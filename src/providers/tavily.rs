use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::{SearchOpts, SearchResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

pub struct Tavily {
    ctx: Arc<AppContext>,
}

impl Tavily {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }

    fn api_key(&self) -> String {
        super::resolve_key(&self.ctx.config.keys.tavily, "TAVILY_API_KEY")
    }

    async fn do_search(
        &self,
        query: &str,
        count: usize,
        topic: &str,
        opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        if self.api_key().is_empty() {
            return Err(SearchError::AuthMissing { provider: "tavily" });
        }

        let mut body = json!({
            "query": query,
            "search_depth": "advanced",
            "topic": topic,
            // The API accepts 0..=20.
            "max_results": count.min(20),
            "include_answer": "basic",
            "include_raw_content": false,
            "include_published_date": true,
        });
        if !opts.include_domains.is_empty() {
            body["include_domains"] = json!(opts.include_domains);
        }
        if !opts.exclude_domains.is_empty() {
            body["exclude_domains"] = json!(opts.exclude_domains);
        }
        // Tavily time_range: day, week, month, year
        if let Some(f) = &opts.freshness {
            body["time_range"] = json!(f);
        }

        let client = &self.ctx.client;
        let key = self.api_key();
        let resp = super::retry_request(|| async {
            let r = client
                .post("https://api.tavily.com/search")
                .bearer_auth(&key)
                .json(&body)
                .send()
                .await?;
            let r = super::ok_or_api_error(r, "tavily").await?;
            Ok(r.json::<TavilyResponse>().await?)
        })
        .await?;

        let source = if topic == "news" {
            "tavily_news"
        } else {
            "tavily"
        };
        let mut results = Vec::new();

        // Prepend the AI-generated answer if available
        if let Some(answer) = resp.answer {
            if !answer.is_empty() {
                results.push(SearchResult {
                    title: "AI Answer".to_string(),
                    url: "tavily://answer".to_string(),
                    snippet: answer,
                    source: format!("{source}_answer"),
                    published: None,
                    image_url: None,
                    extra: None,
                });
            }
        }

        results.extend(resp.results.into_iter().map(|r| SearchResult {
            title: r.title.unwrap_or_default(),
            url: r.url.unwrap_or_default(),
            snippet: r.content.unwrap_or_default(),
            source: source.to_string(),
            published: r.published_date,
            image_url: None,
            extra: r.score.map(|s| json!({"score": s})),
        }));
        Ok(results)
    }
}

#[derive(Deserialize)]
struct TavilyResponse {
    results: Vec<TavilyResult>,
    answer: Option<String>,
}

#[derive(Deserialize)]
struct TavilyResult {
    title: Option<String>,
    url: Option<String>,
    content: Option<String>,
    score: Option<f64>,
    published_date: Option<String>,
}

#[async_trait]
impl super::Provider for Tavily {
    fn name(&self) -> &'static str {
        "tavily"
    }
    fn capabilities(&self) -> &[&'static str] {
        &["general", "news", "academic", "deep"]
    }
    fn env_keys(&self) -> &[&'static str] {
        &["TAVILY_API_KEY", "SEARCH_KEYS_TAVILY"]
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
        self.do_search(query, count, "general", opts).await
    }

    async fn search_news(
        &self,
        query: &str,
        count: usize,
        opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        self.do_search(query, count, "news", opts).await
    }
}

#[cfg(test)]
mod tests {
    use super::TavilyResponse;

    #[test]
    fn parses_published_date() {
        let body = r#"{"answer": null, "results": [{"title": "t", "url": "https://e.com",
            "content": "c", "score": 0.9, "published_date": "Fri, 09 Oct 2026 15:02:56 GMT"}]}"#;
        let resp: TavilyResponse = serde_json::from_str(body).unwrap();
        assert_eq!(
            resp.results[0].published_date.as_deref(),
            Some("Fri, 09 Oct 2026 15:02:56 GMT")
        );
    }
}
