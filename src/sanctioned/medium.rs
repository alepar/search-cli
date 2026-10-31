//! Medium posts through the author's or publication's RSS feed, which carries
//! full post bodies while the pages sit behind Cloudflare. Feeds hold only
//! recent posts (about ten), so an older post falls through to the chain.

use super::{get, host_is, html_fragment_to_text, page, Route};
use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::SearchResult;
use async_trait::async_trait;
use std::sync::Arc;
use url::Url;

const NAME: &str = "medium";
const CONTENT_NS: &str = "http://purl.org/rss/1.0/modules/content/";
const DC_NS: &str = "http://purl.org/dc/elements/1.1/";

pub struct Medium {
    ctx: Arc<AppContext>,
}

impl Medium {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }
}

/// Feed URL and post id for a Medium post URL.
fn parse(url: &Url) -> Option<(String, String)> {
    if !host_is(url, "medium.com") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    let segs: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let id = post_id(segs.last()?)?;
    let feed = match (host.as_str(), segs.as_slice()) {
        ("medium.com" | "www.medium.com", [owner, _slug]) if *owner != "p" => {
            format!("https://medium.com/feed/{owner}")
        }
        (sub, [_slug]) if sub != "medium.com" && sub != "www.medium.com" => {
            format!("https://{sub}/feed")
        }
        _ => return None,
    };
    Some((feed, id))
}

/// Medium slugs end in a hex post id: "my-title-0123abcd4567".
fn post_id(slug: &str) -> Option<String> {
    let id = slug.rsplit('-').next()?;
    (8..=16)
        .contains(&id.len())
        .then_some(id)
        .filter(|id| id.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_string)
}

struct Post {
    title: String,
    link: String,
    author: Option<String>,
    published: Option<String>,
    html: String,
}

fn find_post(feed_xml: &str, id: &str) -> Result<Option<Post>, String> {
    let doc = roxmltree::Document::parse(feed_xml).map_err(|e| e.to_string())?;
    let child_text = |item: roxmltree::Node, ns: Option<&str>, local: &str| {
        item.children()
            .find(|c| c.tag_name().name() == local && c.tag_name().namespace() == ns)
            .and_then(|c| c.text())
            .map(str::to_string)
    };
    for item in doc.descendants().filter(|n| n.has_tag_name("item")) {
        let guid = child_text(item, None, "guid").unwrap_or_default();
        let link = child_text(item, None, "link").unwrap_or_default();
        if !guid.ends_with(&format!("/p/{id}")) && !link.contains(id) {
            continue;
        }
        return Ok(Some(Post {
            title: child_text(item, None, "title").unwrap_or_default(),
            link: link.split('?').next().unwrap_or_default().to_string(),
            author: child_text(item, Some(DC_NS), "creator"),
            published: child_text(item, None, "pubDate"),
            html: child_text(item, Some(CONTENT_NS), "encoded").unwrap_or_default(),
        }));
    }
    Ok(None)
}

#[async_trait]
impl Route for Medium {
    fn name(&self) -> &'static str {
        NAME
    }

    fn matches(&self, url: &Url) -> bool {
        parse(url).is_some()
    }

    fn fallback_to_chain(&self) -> bool {
        true
    }

    async fn fetch(&self, url: &Url) -> Result<Vec<SearchResult>, SearchError> {
        let (feed, id) = parse(url).ok_or_else(|| SearchError::InvalidInput {
            message: format!("not a Medium post URL: {url}"),
        })?;
        let xml = get(&self.ctx, NAME, &feed, "application/rss+xml")
            .await?
            .text()
            .await?;
        let post = find_post(&xml, &id).map_err(|e| SearchError::Api {
            provider: NAME,
            code: "json_error",
            message: format!("unreadable feed {feed}: {e}"),
            status: None,
        })?;
        // Not in the feed's recent window: no results, the chain takes over.
        let Some(post) = post else {
            return Ok(vec![]);
        };
        let text = format!("# {}\n\n{}", post.title, html_fragment_to_text(&post.html));
        let extra = serde_json::json!({ "author": post.author, "feed": feed });
        let mut result = page(NAME, post.title, post.link, text, extra);
        result.published = post.published;
        Ok(vec![result])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(u: &str) -> Option<(String, String)> {
        parse(&Url::parse(u).unwrap())
    }

    #[test]
    fn feed_urls_derive_from_post_urls() {
        assert_eq!(
            p("https://medium.com/airbnb-engineering/a-title-0123abcd4567"),
            Some((
                "https://medium.com/feed/airbnb-engineering".into(),
                "0123abcd4567".into()
            ))
        );
        assert_eq!(
            p("https://medium.com/@someone/hello-world-89ab01cd"),
            Some(("https://medium.com/feed/@someone".into(), "89ab01cd".into()))
        );
        assert_eq!(
            p("https://someone.medium.com/hello-world-89ab01cd?source=x"),
            Some(("https://someone.medium.com/feed".into(), "89ab01cd".into()))
        );
        assert_eq!(p("https://medium.com/p/89ab01cd"), None);
        assert_eq!(p("https://medium.com/airbnb-engineering"), None);
        assert_eq!(p("https://medium.com/tag/rust-lang"), None);
    }

    #[test]
    fn finds_post_in_feed_by_id() {
        let xml = r#"<?xml version="1.0"?>
<rss xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:content="http://purl.org/rss/1.0/modules/content/"><channel>
<item><title>Other</title><link>https://medium.com/x/other-111111111111</link><guid>https://medium.com/p/111111111111</guid></item>
<item><title>Wanted</title><link>https://medium.com/x/wanted-0123abcd4567?source=rss</link>
<guid isPermaLink="false">https://medium.com/p/0123abcd4567</guid><dc:creator>Ann</dc:creator>
<pubDate>Tue, 01 Sep 2026 10:00:00 GMT</pubDate><content:encoded><![CDATA[<p>Body &amp; soul</p>]]></content:encoded></item>
</channel></rss>"#;
        let post = find_post(xml, "0123abcd4567").unwrap().unwrap();
        assert_eq!(post.title, "Wanted");
        assert_eq!(post.link, "https://medium.com/x/wanted-0123abcd4567");
        assert_eq!(post.author.as_deref(), Some("Ann"));
        assert_eq!(html_fragment_to_text(&post.html), "Body & soul");
        assert!(find_post(xml, "fedcba987654").unwrap().is_none());
    }
}
