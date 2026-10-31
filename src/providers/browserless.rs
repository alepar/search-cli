use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::{SearchOpts, SearchResult};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

pub struct Browserless {
    ctx: Arc<AppContext>,
}

impl Browserless {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }

    fn api_key(&self) -> String {
        super::resolve_key(&self.ctx.config.keys.browserless, "BROWSERLESS_API_KEY")
    }

    /// Scrape a URL with Browserless's cloud browser (JS rendering)
    pub async fn scrape_url(&self, url: &str) -> Result<Vec<SearchResult>, SearchError> {
        if self.api_key().is_empty() {
            return Err(SearchError::AuthMissing {
                provider: "browserless",
            });
        }

        // bestAttempt: return what rendered if the selector wait times out,
        // rather than failing the whole rung.
        let endpoint = "https://production-sfo.browserless.io/content";

        let body = serde_json::json!({
            "url": url,
            "waitForSelector": { "selector": "body", "timeout": 10000 },
            "bestAttempt": true
        });

        let client = &self.ctx.client;
        let token = self.api_key().to_string();

        let resp = super::retry_request(|| async {
            let r = client
                .post(endpoint)
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .json(&body)
                .send()
                .await?;

            if r.status() == 429 {
                return Err(SearchError::RateLimited {
                    provider: "browserless",
                });
            }
            let r = super::ok_or_api_error(r, "browserless").await?;

            Ok(r.text().await?)
        })
        .await?;

        // resp is fully rendered HTML — extract with readability
        let parsed_url = url::Url::parse(url).map_err(|e| SearchError::Api {
            provider: "browserless",
            code: "invalid_url",
            message: format!("Invalid URL '{}': {}", url, e),
            status: None,
        })?;

        let mut cursor = std::io::Cursor::new(resp.as_bytes());
        let (title, text) = match readability::extractor::extract(&mut cursor, &parsed_url) {
            Ok(article) if !article.text.trim().is_empty() => {
                let title = if article.title.is_empty() {
                    url.to_string()
                } else {
                    article.title
                };
                (title, article.text)
            }
            _ => (url.to_string(), super::html_to_text(&resp)),
        };

        if text.trim().is_empty() {
            return Err(SearchError::Api {
                provider: "browserless",
                code: "extraction_error",
                message: "Page returned no extractable content".to_string(),
                status: None,
            });
        }

        Ok(vec![SearchResult {
            title,
            url: url.to_string(),
            snippet: text,
            source: "browserless".to_string(),
            published: None,
            image_url: None,
            extra: None,
        }])
    }
}

#[async_trait]
impl super::Provider for Browserless {
    fn name(&self) -> &'static str {
        "browserless"
    }

    fn env_keys(&self) -> &[&'static str] {
        &["BROWSERLESS_API_KEY", "SEARCH_KEYS_BROWSERLESS"]
    }
    fn capabilities(&self) -> &[&'static str] {
        &["scrape", "extract"]
    }

    fn is_configured(&self) -> bool {
        !self.api_key().is_empty()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    async fn search(
        &self,
        _query: &str,
        _count: usize,
        _opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        Ok(vec![])
    }

    async fn search_news(
        &self,
        _query: &str,
        _count: usize,
        _opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        Ok(vec![])
    }
}
