//! Hacker News items (story + comment tree) through the Algolia HN API,
//! one keyless request instead of a page per comment level.

use super::{get, html_fragment_to_text, page, Route};
use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::SearchResult;
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;
use url::Url;

const NAME: &str = "hackernews";
/// Keep huge threads to a size an agent can read.
const MAX_COMMENTS: usize = 200;
const MAX_DEPTH: usize = 6;

pub struct HackerNews {
    ctx: Arc<AppContext>,
}

impl HackerNews {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }
}

fn item_id(url: &Url) -> Option<u64> {
    if url.host_str()? != "news.ycombinator.com" || url.path() != "/item" {
        return None;
    }
    url.query_pairs()
        .find(|(k, _)| k == "id")
        .and_then(|(_, v)| v.parse().ok())
}

#[derive(Deserialize)]
struct Item {
    title: Option<String>,
    url: Option<String>,
    text: Option<String>,
    author: Option<String>,
    points: Option<i64>,
    created_at: Option<String>,
    #[serde(default)]
    children: Vec<Item>,
}

fn render_comments(items: &[Item], depth: usize, budget: &mut usize, out: &mut String) {
    for c in items {
        if *budget == 0 {
            return;
        }
        let Some(text) = c.text.as_deref() else {
            continue; // deleted or dead
        };
        *budget -= 1;
        let indent = "  ".repeat(depth);
        out.push_str(&format!(
            "\n{indent}- {}: {}\n",
            c.author.as_deref().unwrap_or("[deleted]"),
            html_fragment_to_text(text).replace('\n', &format!("\n{indent}  "))
        ));
        if depth + 1 < MAX_DEPTH {
            render_comments(&c.children, depth + 1, budget, out);
        }
    }
}

#[async_trait]
impl Route for HackerNews {
    fn name(&self) -> &'static str {
        NAME
    }

    fn matches(&self, url: &Url) -> bool {
        item_id(url).is_some()
    }

    async fn fetch(&self, url: &Url) -> Result<Vec<SearchResult>, SearchError> {
        let id = item_id(url).ok_or_else(|| SearchError::InvalidInput {
            message: format!("not a Hacker News item URL: {url}"),
        })?;
        let api = format!("https://hn.algolia.com/api/v1/items/{id}");
        let item: Item = get(&self.ctx, NAME, &api, "application/json")
            .await?
            .json()
            .await?;
        let title = item
            .title
            .clone()
            .unwrap_or_else(|| format!("HN item {id}"));
        let mut text = format!(
            "# {title}\n\nby {} ({} points){}\n",
            item.author.as_deref().unwrap_or("[deleted]"),
            item.points.unwrap_or(0),
            item.url
                .as_deref()
                .map(|u| format!("\nLink: {u}"))
                .unwrap_or_default(),
        );
        if let Some(body) = item.text.as_deref() {
            text.push_str(&format!("\n{}\n", html_fragment_to_text(body)));
        }
        let mut budget = MAX_COMMENTS;
        render_comments(&item.children, 0, &mut budget, &mut text);
        let extra = serde_json::json!({
            "points": item.points,
            "created_at": item.created_at,
            "linked_url": item.url,
            "comments_truncated": budget == 0,
        });
        Ok(vec![page(NAME, title, url.to_string(), text, extra)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_ids_parse_only_from_item_pages() {
        let id = |u: &str| item_id(&Url::parse(u).unwrap());
        assert_eq!(
            id("https://news.ycombinator.com/item?id=38773487"),
            Some(38773487)
        );
        assert_eq!(id("https://news.ycombinator.com/news"), None);
        assert_eq!(id("https://news.ycombinator.com/item?id=abc"), None);
    }

    #[test]
    fn comments_render_nested_and_respect_budget() {
        let leaf = |a: &str, t: &str| Item {
            title: None,
            url: None,
            text: Some(t.into()),
            author: Some(a.into()),
            points: None,
            created_at: None,
            children: vec![],
        };
        let mut parent = leaf("a", "<p>top</p>");
        parent.children = vec![leaf("b", "reply"), leaf("c", "second")];
        let mut out = String::new();
        let mut budget = 2;
        render_comments(&[parent], 0, &mut budget, &mut out);
        assert_eq!(out, "\n- a: top\n\n  - b: reply\n");
        assert_eq!(budget, 0);
    }
}
