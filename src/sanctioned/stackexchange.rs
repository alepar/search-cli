//! Stack Exchange questions through API v2.3. The HTML sits behind a
//! Cloudflare challenge; the API answers keyless (300 requests/day per IP,
//! 10,000 with a free stackapps key in STACKEXCHANGE_KEY).

use super::{get, host_is, html_fragment_to_text, page, Route};
use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::SearchResult;
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;
use url::Url;

const NAME: &str = "stackexchange";
const MAX_ANSWERS: usize = 10;

/// Sites served by the network API that don't live under stackexchange.com.
const SITES: &[&str] = &[
    "stackoverflow.com",
    "superuser.com",
    "serverfault.com",
    "askubuntu.com",
    "mathoverflow.net",
    "stackapps.com",
    "stackexchange.com",
];

pub struct StackExchange {
    ctx: Arc<AppContext>,
}

impl StackExchange {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }
}

/// The API `site` parameter (it accepts the domain) and the question id.
fn parse(url: &Url) -> Option<(String, u64)> {
    if !SITES.iter().any(|d| host_is(url, d)) {
        return None;
    }
    let site = url.host_str()?.trim_start_matches("www.").to_string();
    let mut segs = url.path_segments()?;
    match segs.next()? {
        "questions" | "q" => Some((site, segs.next()?.parse().ok()?)),
        _ => None,
    }
}

#[derive(Deserialize)]
struct Envelope<T> {
    #[serde(default = "Vec::new")]
    items: Vec<T>,
    quota_remaining: Option<u32>,
}

#[derive(Deserialize)]
struct Question {
    title: String,
    link: String,
    body: Option<String>,
    score: i64,
    #[serde(default)]
    tags: Vec<String>,
    owner: Option<Owner>,
    creation_date: Option<i64>,
}

#[derive(Deserialize)]
struct Answer {
    body: Option<String>,
    score: i64,
    #[serde(default)]
    is_accepted: bool,
    owner: Option<Owner>,
}

#[derive(Deserialize)]
struct Owner {
    display_name: Option<String>,
}

fn author(owner: &Option<Owner>) -> String {
    owner
        .as_ref()
        .and_then(|o| o.display_name.as_deref())
        .map(super::decode_entities)
        .unwrap_or_else(|| "unknown".into())
}

impl StackExchange {
    async fn call<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        site: &str,
        extra: &str,
    ) -> Result<Envelope<T>, SearchError> {
        let key = crate::providers::resolve_key("", "STACKEXCHANGE_KEY");
        let mut url =
            format!("https://api.stackexchange.com/2.3/{path}?site={site}&filter=withbody{extra}");
        if !key.is_empty() {
            url.push_str(&format!("&key={key}"));
        }
        let resp = get(&self.ctx, NAME, &url, "application/json").await?;
        Ok(resp.json().await?)
    }
}

#[async_trait]
impl Route for StackExchange {
    fn name(&self) -> &'static str {
        NAME
    }

    fn matches(&self, url: &Url) -> bool {
        parse(url).is_some()
    }

    async fn fetch(&self, url: &Url) -> Result<Vec<SearchResult>, SearchError> {
        let (site, id) = parse(url).ok_or_else(|| SearchError::InvalidInput {
            message: format!("not a Stack Exchange question URL: {url}"),
        })?;
        let q: Envelope<Question> = self.call(&format!("questions/{id}"), &site, "").await?;
        let Some(question) = q.items.into_iter().next() else {
            return Ok(vec![]);
        };
        let answers: Envelope<Answer> = self
            .call(
                &format!("questions/{id}/answers"),
                &site,
                &format!("&sort=votes&order=desc&pagesize={MAX_ANSWERS}"),
            )
            .await?;

        let title = super::decode_entities(&question.title);
        let mut text = format!(
            "# {title}\n\nQuestion by {} (score {}){}\n\n{}\n",
            author(&question.owner),
            question.score,
            if question.tags.is_empty() {
                String::new()
            } else {
                format!(", tags: {}", question.tags.join(", "))
            },
            html_fragment_to_text(question.body.as_deref().unwrap_or_default()),
        );
        for a in &answers.items {
            text.push_str(&format!(
                "\n## Answer by {} (score {}{})\n\n{}\n",
                author(&a.owner),
                a.score,
                if a.is_accepted { ", accepted" } else { "" },
                html_fragment_to_text(a.body.as_deref().unwrap_or_default()),
            ));
        }
        let extra = serde_json::json!({
            "license": "CC BY-SA 4.0",
            "attribution": format!("{} via the Stack Exchange API", question.link),
            "answers_included": answers.items.len(),
            "created_unix": question.creation_date,
            "quota_remaining": answers.quota_remaining.or(q.quota_remaining),
        });
        Ok(vec![page(NAME, title, question.link, text, extra)])
    }
}

#[cfg(test)]
mod tests {
    use super::parse;
    use url::Url;

    fn p(u: &str) -> Option<(String, u64)> {
        parse(&Url::parse(u).unwrap())
    }

    #[test]
    fn parses_question_urls_across_the_network() {
        assert_eq!(
            p("https://stackoverflow.com/questions/11227809/why-is-processing"),
            Some(("stackoverflow.com".into(), 11227809))
        );
        assert_eq!(
            p("https://unix.stackexchange.com/q/42/"),
            Some(("unix.stackexchange.com".into(), 42))
        );
        assert_eq!(
            p("https://www.superuser.com/questions/7/x"),
            Some(("superuser.com".into(), 7))
        );
        assert_eq!(p("https://stackoverflow.com/questions/tagged/rust"), None);
        assert_eq!(p("https://stackoverflow.com/users/1/x"), None);
        assert_eq!(p("https://example.com/questions/1"), None);
    }
}
