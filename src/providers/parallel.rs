use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::{SearchOpts, SearchResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub struct Parallel {
    ctx: Arc<AppContext>,
}

impl Parallel {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }

    fn api_key(&self) -> String {
        super::resolve_key(&self.ctx.config.keys.parallel, "PARALLEL_API_KEY")
    }
}

#[derive(Deserialize)]
struct ParallelResponse {
    results: Option<Vec<ParallelResult>>,
}

#[derive(Deserialize)]
struct ParallelResult {
    title: Option<String>,
    url: Option<String>,
    excerpts: Option<Vec<String>>,
    publish_date: Option<String>,
}

/// Request body for POST /v1/search. v1 rejects unknown fields, and every
/// tuning knob (result count, excerpt size, location, source policy) lives
/// under `advanced_settings`.
fn build_body(query: &str, count: usize, opts: &SearchOpts) -> serde_json::Value {
    let mut source_policy = serde_json::Map::new();
    if !opts.include_domains.is_empty() {
        source_policy.insert("include_domains".to_string(), json!(opts.include_domains));
    }
    if !opts.exclude_domains.is_empty() {
        source_policy.insert("exclude_domains".to_string(), json!(opts.exclude_domains));
    }
    if let Some(days) = opts.freshness.as_deref().and_then(super::freshness_days) {
        source_policy.insert("after_date".to_string(), json!(super::date_days_ago(days)));
    }

    let mut advanced = json!({
        "max_results": count,
        "excerpt_settings": { "max_chars_per_result": 3000 },
    });
    if !source_policy.is_empty() {
        advanced["source_policy"] = json!(source_policy);
    }
    if let Some(country) = &opts.country {
        advanced["location"] = json!(country.to_lowercase());
    }
    json!({
        "objective": query,
        "search_queries": [query],
        "mode": "fast",
        "advanced_settings": advanced,
    })
}

#[async_trait]
impl super::Provider for Parallel {
    fn name(&self) -> &'static str {
        "parallel"
    }

    fn capabilities(&self) -> &[&'static str] {
        &["general", "news", "deep"]
    }

    fn env_keys(&self) -> &[&'static str] {
        &["PARALLEL_API_KEY", "SEARCH_KEYS_PARALLEL"]
    }

    fn is_configured(&self) -> bool {
        !self.api_key().is_empty()
    }

    async fn search(
        &self,
        query: &str,
        count: usize,
        opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        if !self.is_configured() {
            return Err(SearchError::AuthMissing {
                provider: "parallel",
            });
        }

        let client = &self.ctx.client;
        let api_key = self.api_key();

        let body = build_body(query, count, opts);

        super::retry_request(|| async {
            let resp = client
                .post("https://api.parallel.ai/v1/search")
                .header("x-api-key", api_key.as_str())
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;

            let resp = super::ok_or_api_error(resp, "parallel").await?;

            let data: ParallelResponse = resp.json().await?;
            let results = data
                .results
                .unwrap_or_default()
                .into_iter()
                .filter_map(|r| {
                    let url = r.url?;
                    Some(SearchResult {
                        title: r.title.unwrap_or_default(),
                        url,
                        snippet: r.excerpts.unwrap_or_default().join("\n\n"),
                        source: "parallel".to_string(),
                        published: r.publish_date,
                        image_url: None,
                        extra: None,
                    })
                })
                .collect();

            Ok(results)
        })
        .await
    }

    async fn search_news(
        &self,
        query: &str,
        count: usize,
        opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        self.search(query, count, opts).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_body_nests_settings_under_advanced_settings() {
        let opts = SearchOpts {
            include_domains: vec!["rust-lang.org".into()],
            country: Some("GB".into()),
            ..SearchOpts::default()
        };
        let body = build_body("rust", 7, &opts);
        assert_eq!(
            body.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["advanced_settings", "mode", "objective", "search_queries"]
        );
        let adv = &body["advanced_settings"];
        assert_eq!(adv["max_results"], 7);
        assert_eq!(adv["excerpt_settings"]["max_chars_per_result"], 3000);
        assert_eq!(adv["location"], "gb");
        assert_eq!(
            adv["source_policy"]["include_domains"],
            json!(["rust-lang.org"])
        );
    }

    #[test]
    fn parses_publish_date() {
        let r: ParallelResponse = serde_json::from_str(
            r#"{"results": [{"url": "https://e.com", "title": "t",
                "publish_date": "2024-01-15", "excerpts": ["a", "b"]}]}"#,
        )
        .unwrap();
        let r = &r.results.unwrap()[0];
        assert_eq!(r.publish_date.as_deref(), Some("2024-01-15"));
    }
}
