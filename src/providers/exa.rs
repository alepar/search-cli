use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::{SearchOpts, SearchResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// Exa documents numResults up to 100.
pub(super) const MAX_COUNT: usize = 100;

pub struct Exa {
    ctx: Arc<AppContext>,
}

impl Exa {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }

    fn api_key(&self) -> String {
        super::resolve_key(&self.ctx.config.keys.exa, "EXA_API_KEY")
    }

    async fn post_api(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<ExaResponse, SearchError> {
        if self.api_key().is_empty() {
            return Err(SearchError::AuthMissing { provider: "exa" });
        }

        let url = format!("https://api.exa.ai/{path}");
        let client = &self.ctx.client;
        let api_key = self.api_key();

        super::retry_request(|| async {
            let resp = client
                .post(&url)
                .header("x-api-key", api_key.as_str())
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;

            let resp = super::ok_or_api_error(resp, "exa").await?;

            let body_bytes = resp.bytes().await?;
            let mut body_vec = body_bytes.to_vec();
            simd_json::from_slice(&mut body_vec).map_err(|e| SearchError::Api {
                provider: "exa",
                code: "json_error",
                status: None,
                message: e.to_string(),
            })
        })
        .await
    }
}

#[derive(Deserialize)]
struct ExaResponse {
    results: Option<Vec<ExaResult>>,
}

#[derive(Deserialize)]
struct ExaResult {
    title: Option<String>,
    url: Option<String>,
    text: Option<String>,
    #[serde(rename = "publishedDate")]
    published_date: Option<String>,
    highlights: Option<Vec<String>>,
}

fn to_results(exa: ExaResponse, source: &str) -> Vec<SearchResult> {
    exa.results
        .unwrap_or_default()
        .into_iter()
        .map(|r| {
            // Prefer highlights over full text for snippet (more relevant, shorter)
            let snippet = if let Some(ref hl) = r.highlights {
                if !hl.is_empty() {
                    hl.join(" ... ")
                } else {
                    r.text.clone().unwrap_or_default()
                }
            } else {
                r.text.unwrap_or_default()
            };
            SearchResult {
                title: r.title.unwrap_or_default(),
                url: r.url.unwrap_or_default(),
                snippet,
                source: source.to_string(),
                published: r.published_date,
                image_url: None,
                extra: None,
            }
        })
        .collect()
}

fn build_search_body(query: &str, count: usize, opts: &SearchOpts) -> serde_json::Value {
    let mut body = json!({
        "query": query,
        "numResults": count.min(MAX_COUNT),
        "type": "auto",
        "contents": {
            "text": true,
            "highlights": true
        }
    });
    // Exa supports includeDomains / excludeDomains natively
    if !opts.include_domains.is_empty() {
        body["includeDomains"] = json!(opts.include_domains);
    }
    if !opts.exclude_domains.is_empty() {
        body["excludeDomains"] = json!(opts.exclude_domains);
    }
    // Freshness → startPublishedDate (ISO 8601)
    if let Some(days) = opts.freshness.as_deref().and_then(super::freshness_days) {
        body["startPublishedDate"] = json!(format!("{}T00:00:00Z", super::date_days_ago(days)));
    }
    body
}

#[async_trait]
impl super::Provider for Exa {
    fn name(&self) -> &'static str {
        "exa"
    }
    fn capabilities(&self) -> &[&'static str] {
        &["general", "academic", "people", "similar", "deep"]
    }
    fn env_keys(&self) -> &[&'static str] {
        &["EXA_API_KEY", "SEARCH_KEYS_EXA"]
    }
    fn is_configured(&self) -> bool {
        !self.api_key().is_empty()
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(15)
    }

    async fn search(
        &self,
        query: &str,
        count: usize,
        opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let body = build_search_body(query, count, opts);
        let resp = self.post_api("search", body).await?;
        Ok(to_results(resp, "exa"))
    }

    async fn search_news(
        &self,
        query: &str,
        count: usize,
        opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let mut body = build_search_body(query, count, opts);
        body["category"] = json!("news");
        let resp = self.post_api("search", body).await?;
        Ok(to_results(resp, "exa_news"))
    }
}

impl Exa {
    pub async fn search_people(
        &self,
        query: &str,
        count: usize,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let body = json!({
            "query": query, "numResults": count.min(MAX_COUNT), "type": "auto",
            "category": "people", "contents": { "text": true }
        });
        let resp = self.post_api("search", body).await?;
        Ok(to_results(resp, "exa_people"))
    }

    pub async fn find_similar(
        &self,
        url: &str,
        count: usize,
    ) -> Result<Vec<SearchResult>, SearchError> {
        // /findSimilar is marked deprecated in Exa's OpenAPI spec (no sunset
        // date), which suggests /search with a query describing the source
        // instead. Checked live (2026-10-31): /search with the URL as query
        // returns pages about the same site rather than similar sites, so
        // this stays on /findSimilar until Exa removes it or ships a
        // like-for-like replacement.
        let body =
            json!({ "url": url, "numResults": count.min(MAX_COUNT), "contents": { "text": true } });
        let resp = self.post_api("findSimilar", body).await?;
        Ok(to_results(resp, "exa_similar"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_preferred_over_text() {
        let resp: ExaResponse = serde_json::from_value(json!({"results": [
            {"title": "a", "url": "https://a.com", "text": "full", "highlights": ["h1", "h2"],
             "publishedDate": "2026-10-01T00:00:00.000Z"},
            {"title": "b", "url": "https://b.com", "text": "full b", "highlights": []}
        ]}))
        .unwrap();
        let r = to_results(resp, "exa");
        assert_eq!(r[0].snippet, "h1 ... h2");
        assert_eq!(r[0].published.as_deref(), Some("2026-10-01T00:00:00.000Z"));
        assert_eq!(r[1].snippet, "full b");
    }

    #[test]
    fn search_body_caps_num_results() {
        let body = build_search_body("rust", 150, &SearchOpts::default());
        assert_eq!(body["numResults"], 100);
    }

    #[test]
    fn search_body_filters() {
        let opts = SearchOpts {
            include_domains: vec!["arxiv.org".into()],
            freshness: Some("week".into()),
            ..SearchOpts::default()
        };
        let body = build_search_body("rust", 5, &opts);
        assert_eq!(body["numResults"], 5);
        assert_eq!(body["includeDomains"], json!(["arxiv.org"]));
        let start = body["startPublishedDate"].as_str().unwrap();
        assert!(
            start.ends_with("T00:00:00Z") && start.len() == 20,
            "{start}"
        );
    }
}
