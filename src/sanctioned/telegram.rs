//! Public Telegram channels through the t.me/s/ web preview, Telegram's own
//! read-only page for non-members. Channels can switch it off.

use super::{decode_entities, get, html_fragment_to_text, page, Route};
use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::SearchResult;
use async_trait::async_trait;
use std::sync::Arc;
use url::Url;

const NAME: &str = "telegram";
/// Paths under t.me that are not channels.
const RESERVED: &[&str] = &[
    "s",
    "joinchat",
    "addstickers",
    "addemoji",
    "share",
    "proxy",
    "socks",
    "login",
    "iv",
    "c",
];

pub struct Telegram {
    ctx: Arc<AppContext>,
}

impl Telegram {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }
}

/// Channel name and optional post id.
fn parse(url: &Url) -> Option<(String, Option<u64>)> {
    let host = url.host_str()?.to_ascii_lowercase();
    if !matches!(host.as_str(), "t.me" | "telegram.me" | "www.t.me") {
        return None;
    }
    let segs: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let (channel, rest) = match segs.as_slice() {
        ["s", channel, rest @ ..] => (*channel, rest),
        [channel, rest @ ..] => (*channel, rest),
        [] => return None,
    };
    if RESERVED.contains(&channel) || channel.starts_with('+') {
        return None;
    }
    let post = match rest {
        [] => None,
        [id] => Some(id.parse().ok()?),
        _ => return None,
    };
    Some((channel.to_string(), post))
}

#[derive(Debug, PartialEq)]
struct Message {
    post: String,
    date: Option<String>,
    text: String,
}

/// Pull messages out of the preview page. The markup is stable and flat:
/// each message block carries data-post="channel/id", a <time datetime>,
/// and a tgme_widget_message_text div without nested divs.
fn messages(html: &str) -> Vec<Message> {
    let mut out = Vec::new();
    for block in html.split("data-post=\"").skip(1) {
        let Some(post) = block.split('"').next() else {
            continue;
        };
        let Some(start) = block.find("tgme_widget_message_text") else {
            continue;
        };
        let body = &block[start..];
        let Some(open_end) = body.find('>') else {
            continue;
        };
        let body = &body[open_end + 1..];
        let inner = &body[..body.find("</div>").unwrap_or(body.len())];
        let date = block
            .find("datetime=\"")
            .map(|i| &block[i + 10..])
            .and_then(|s| s.split('"').next())
            .map(str::to_string);
        out.push(Message {
            post: post.to_string(),
            date,
            text: html_fragment_to_text(inner),
        });
    }
    out
}

fn og_title(html: &str) -> Option<String> {
    let i = html.find("property=\"og:title\" content=\"")?;
    let rest = &html[i + 29..];
    Some(decode_entities(rest.split('"').next()?))
}

#[async_trait]
impl Route for Telegram {
    fn name(&self) -> &'static str {
        NAME
    }

    fn matches(&self, url: &Url) -> bool {
        parse(url).is_some()
    }

    async fn fetch(&self, url: &Url) -> Result<Vec<SearchResult>, SearchError> {
        let (channel, post) = parse(url).ok_or_else(|| SearchError::InvalidInput {
            message: format!("not a public Telegram channel URL: {url}"),
        })?;
        let preview = match post {
            // The preview of a post shows it with its neighbours.
            Some(id) => format!("https://t.me/s/{channel}/{id}"),
            None => format!("https://t.me/s/{channel}"),
        };
        let html = get(&self.ctx, NAME, &preview, "text/html")
            .await?
            .text()
            .await?;
        let mut msgs = messages(&html);
        if let Some(id) = post {
            let want = format!("{channel}/{id}");
            msgs.retain(|m| m.post.eq_ignore_ascii_case(&want));
        }
        if msgs.is_empty() {
            return Err(SearchError::Blocked {
                provider: NAME,
                code: "no_sanctioned_route",
                message: format!(
                    "no public web preview for t.me/{channel}{}: the channel is private, has the preview switched off, or the post does not exist",
                    post.map(|p| format!("/{p}")).unwrap_or_default()
                ),
            });
        }
        let title = og_title(&html).unwrap_or_else(|| channel.clone());
        let mut text = format!("# {title}\n");
        for m in &msgs {
            text.push_str(&format!(
                "\n## t.me/{}{}\n\n{}\n",
                m.post,
                m.date
                    .as_deref()
                    .map(|d| format!(" ({d})"))
                    .unwrap_or_default(),
                m.text
            ));
        }
        let extra = serde_json::json!({ "channel": channel, "messages": msgs.len() });
        Ok(vec![page(NAME, title, url.to_string(), text, extra)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_channel_and_post_urls() {
        let p = |u: &str| parse(&Url::parse(u).unwrap());
        assert_eq!(p("https://t.me/durov"), Some(("durov".into(), None)));
        assert_eq!(
            p("https://t.me/durov/123"),
            Some(("durov".into(), Some(123)))
        );
        assert_eq!(p("https://t.me/s/durov"), Some(("durov".into(), None)));
        assert_eq!(p("https://t.me/joinchat/abc"), None);
        assert_eq!(p("https://t.me/+abcdef"), None);
        assert_eq!(p("https://t.me/"), None);
        assert_eq!(p("https://example.com/durov"), None);
    }

    #[test]
    fn extracts_messages_from_preview_markup() {
        let html = r#"<meta property="og:title" content="Durov&#39;s Channel">
<div class="tgme_widget_message" data-post="durov/1">
  <div class="tgme_widget_message_text js-message_text" dir="auto">Hello<br/>world &amp; all</div>
  <time datetime="2026-10-01T10:00:00+00:00">Oct 1</time></div>
<div class="tgme_widget_message" data-post="durov/2"><div class="media"></div></div>"#;
        assert_eq!(og_title(html).as_deref(), Some("Durov's Channel"));
        let m = messages(html);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].post, "durov/1");
        assert_eq!(m[0].text, "Hello\nworld & all");
        assert_eq!(m[0].date.as_deref(), Some("2026-10-01T10:00:00+00:00"));
    }
}
