//! Sanctioned routes for extract/scrape: sites whose HTML is walled off but
//! which publish an API, feed or preview meant for programmatic reading. A
//! matching route runs before the generic scraper chain and, unless it opts
//! into fallback, replaces it: retrying a walled site through scrapers only
//! collects block pages.

use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::SearchResult;
use crate::walls;
use async_trait::async_trait;
use std::sync::Arc;
use url::Url;

mod github;
mod hackernews;
mod medium;
mod stackexchange;
mod telegram;
mod walled;
mod youtube;

#[async_trait]
pub trait Route: Send + Sync {
    /// Provider name recorded in the response metadata. `-p` honours it.
    fn name(&self) -> &'static str;
    fn matches(&self, url: &Url) -> bool;
    async fn fetch(&self, url: &Url) -> Result<Vec<SearchResult>, SearchError>;
    /// On failure or no results, run the generic scraper chain afterwards?
    fn fallback_to_chain(&self) -> bool {
        false
    }
}

/// The routing table, first match wins. `walled` stays last so a site with a
/// real route is never shadowed by it.
pub fn routes(ctx: Arc<AppContext>) -> Vec<Box<dyn Route>> {
    vec![
        Box::new(stackexchange::StackExchange::new(ctx.clone())),
        Box::new(github::GitHub::new(ctx.clone())),
        Box::new(hackernews::HackerNews::new(ctx.clone())),
        Box::new(medium::Medium::new(ctx.clone())),
        Box::new(telegram::Telegram::new(ctx.clone())),
        Box::new(youtube::YouTube::new(ctx)),
        Box::new(walled::Walled),
    ]
}

pub fn find<'a>(routes: &'a [Box<dyn Route>], url: &Url) -> Option<&'a dyn Route> {
    routes.iter().find(|r| r.matches(url)).map(|r| r.as_ref())
}

/// True when `url`'s host is `domain` or a subdomain of it.
pub(crate) fn host_is(url: &Url, domain: &str) -> bool {
    url.host_str().is_some_and(|h| {
        let h = h.to_ascii_lowercase();
        h == domain || h.ends_with(&format!(".{domain}"))
    })
}

/// GET through the shared (honestly identified) client. A non-2xx answer
/// that carries a bot-protection signature becomes `SearchError::Blocked`;
/// anything else goes through the usual API error mapping.
pub(crate) async fn get(
    ctx: &AppContext,
    provider: &'static str,
    url: &str,
    accept: &str,
) -> Result<reqwest::Response, SearchError> {
    send(provider, url, ctx.client.get(url).header("Accept", accept)).await
}

/// [`get`] for a request the caller built (extra headers, auth).
pub(crate) async fn send(
    provider: &'static str,
    url: &str,
    req: reqwest::RequestBuilder,
) -> Result<reqwest::Response, SearchError> {
    let resp = req.send().await?;
    let status = resp.status().as_u16();
    if !resp.status().is_success() {
        let headers = resp.headers();
        let wall = walls::classify_response(status, |name| {
            let values: Vec<&str> = headers
                .get_all(name)
                .iter()
                .filter_map(|v| v.to_str().ok())
                .collect();
            (!values.is_empty()).then(|| values.join("; "))
        });
        if let Some(wall) = wall {
            return Err(SearchError::Blocked {
                provider,
                code: wall.code(),
                message: format!("{url} answered HTTP {status} with a {}", wall.code()),
            });
        }
    }
    crate::providers::ok_or_api_error(resp, provider).await
}

/// One extracted page as a result.
pub(crate) fn page(
    provider: &str,
    title: String,
    url: String,
    content: String,
    extra: serde_json::Value,
) -> SearchResult {
    SearchResult {
        title,
        url,
        snippet: content,
        source: provider.to_string(),
        published: None,
        image_url: None,
        extra: Some(extra),
    }
}

/// HTML fragment (API bodies, feed items) to readable text. Unlike
/// `providers::html_to_text` it keeps paragraph breaks and code blocks, and
/// decodes entities: on Stack Overflow the code is the content.
pub(crate) fn html_fragment_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    let mut in_pre = false;
    while let Some(lt) = rest.find('<') {
        push_text(&mut out, &rest[..lt], in_pre);
        let Some(gt) = rest[lt..].find('>') else {
            rest = &rest[lt..];
            break;
        };
        let tag = rest[lt + 1..lt + gt].trim().to_ascii_lowercase();
        rest = &rest[lt + gt + 1..];
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        match name.as_str() {
            "pre" => {
                in_pre = !closing;
                out.push_str("\n```\n");
            }
            "script" | "style" if !closing => {
                let end = format!("</{name}");
                rest = rest
                    .to_ascii_lowercase()
                    .find(&end)
                    .map_or("", |i| &rest[i..]);
            }
            "br" => out.push('\n'),
            "li" if !closing => out.push_str("\n- "),
            "code" if !in_pre => out.push('`'),
            "p" | "div" | "ul" | "ol" | "blockquote" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            | "tr" | "table" | "hr" => out.push_str("\n\n"),
            _ => {}
        }
    }
    push_text(&mut out, rest, in_pre);
    tidy(&out)
}

fn push_text(out: &mut String, text: &str, in_pre: bool) {
    let decoded = decode_entities(text);
    if in_pre {
        out.push_str(&decoded);
    } else {
        // Outside <pre>, source newlines and runs of spaces are layout only.
        let mut last_space = out.ends_with([' ', '\n']);
        for c in decoded.chars() {
            if c.is_whitespace() {
                if !last_space {
                    out.push(' ');
                }
                last_space = true;
            } else {
                out.push(c);
                last_space = false;
            }
        }
    }
}

/// Trim trailing spaces and collapse 3+ newlines to a blank line.
fn tidy(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank_run = 0;
    for line in s.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

pub(crate) fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..semi];
        let ch = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_is_matches_domain_and_subdomains_only() {
        let u = Url::parse("https://math.stackexchange.com/q/1").unwrap();
        assert!(host_is(&u, "stackexchange.com"));
        assert!(!host_is(&u, "exchange.com"));
        let u = Url::parse("https://notgithub.com/").unwrap();
        assert!(!host_is(&u, "github.com"));
    }

    #[test]
    fn html_fragment_keeps_code_and_paragraphs() {
        let html = "<p>Use <code>Vec&lt;T&gt;</code>:</p>\n<pre><code>let v = vec![1, 2];\nv.len();\n</code></pre><p>Done &amp; dusted.</p>";
        let text = html_fragment_to_text(html);
        assert_eq!(
            text,
            "Use `Vec<T>`:\n\n```\nlet v = vec![1, 2];\nv.len();\n\n```\n\nDone & dusted."
        );
    }

    #[test]
    fn html_fragment_drops_scripts_and_lists_items() {
        let html = "<ul><li>one</li><li>two</li></ul><script>var x = '<p>';</script>after";
        assert_eq!(html_fragment_to_text(html), "- one\n- two\n\nafter");
    }

    #[test]
    fn entities_decode_named_and_numeric() {
        assert_eq!(
            decode_entities("a &#39;b&#x27; &lt;c&gt; &bogus; &"),
            "a 'b' <c> &bogus; &"
        );
    }

    #[test]
    fn routing_table_matches_expected_sites() {
        let ctx = Arc::new(AppContext::new(crate::config::AppConfig::default()));
        let table = routes(ctx);
        let name = |u: &str| find(&table, &Url::parse(u).unwrap()).map(|r| r.name());
        assert_eq!(
            name("https://stackoverflow.com/questions/11227809/why-is-it"),
            Some("stackexchange")
        );
        assert_eq!(name("https://github.com/rust-lang/rust"), Some("github"));
        assert_eq!(
            name("https://news.ycombinator.com/item?id=38773487"),
            Some("hackernews")
        );
        assert_eq!(
            name("https://medium.com/airbnb-engineering/some-post-0123abcd4567"),
            Some("medium")
        );
        assert_eq!(name("https://t.me/durov"), Some("telegram"));
        assert_eq!(name("https://youtu.be/dQw4w9WgXcQ"), Some("youtube"));
        assert_eq!(name("https://www.linkedin.com/in/someone"), Some("walled"));
        assert_eq!(name("https://en.wikipedia.org/wiki/Rust"), None);
        // Paths a route cannot serve fall through to the chain.
        assert_eq!(name("https://stackoverflow.com/tags"), None);
    }
}
