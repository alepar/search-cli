use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::{SearchOpts, SearchResult};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use url::Url;
use wreq_util::Profile;

pub struct Stealth {
    _ctx: Arc<AppContext>,
}

impl Stealth {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { _ctx: ctx }
    }

    /// A wreq client with a current Chrome TLS/HTTP2 fingerprint. The
    /// emulation profile supplies Chrome's own default headers, so nothing
    /// is layered on top: hand-written client hints went stale and
    /// contradicted the profile.
    fn build_client() -> Result<wreq::Client, SearchError> {
        wreq::Client::builder()
            .emulation(Profile::Chrome149)
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| SearchError::Config(format!("Failed to build stealth client: {e}")))
    }

    pub async fn scrape_url(&self, url_str: &str) -> Result<Vec<SearchResult>, SearchError> {
        let client = Self::build_client()?;
        let url =
            Url::parse(url_str).map_err(|e| SearchError::Config(format!("Invalid URL: {e}")))?;

        // wreq's IntoUri takes &str (not url::Url); `url` is still used below.
        let req = client.get(url_str);

        let resp = req
            .send()
            .await
            .map_err(|e| SearchError::Config(format!("Stealth request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            return Err(SearchError::Api {
                provider: "stealth",
                code: "api_error",
                message: format!("HTTP {status}"),
                status: Some(status),
            });
        }

        let final_url = url_str.to_string(); // use original URL (wreq may not expose final URL)
        let html_bytes = resp
            .bytes()
            .await
            .map_err(|e| SearchError::Config(format!("Failed to read body: {e}")))?;
        let html = String::from_utf8_lossy(&html_bytes).into_owned();

        // Try readability first, fallback to raw text extraction
        let (title, text) = {
            let mut cursor = std::io::Cursor::new(html.as_bytes());
            match readability::extractor::extract(&mut cursor, &url) {
                Ok(article) if !article.text.trim().is_empty() => {
                    let title = if article.title.is_empty() {
                        url_str.to_string()
                    } else {
                        article.title
                    };
                    (title, article.text)
                }
                _ => {
                    // Readability failed or returned empty — fallback
                    (url_str.to_string(), super::html_to_text(&html))
                }
            }
        };

        if text.trim().is_empty() {
            return Err(SearchError::Api {
                provider: "stealth",
                code: "extraction_error",
                message: "Page returned no extractable text content".to_string(),
                status: None,
            });
        }

        Ok(vec![SearchResult {
            title,
            url: final_url,
            snippet: text,
            source: "stealth".to_string(),
            published: None,
            image_url: None,
            extra: None,
        }])
    }
}

#[async_trait]
impl super::Provider for Stealth {
    fn name(&self) -> &'static str {
        "stealth"
    }

    fn capabilities(&self) -> &[&'static str] {
        &["scrape", "extract"]
    }

    fn env_keys(&self) -> &[&'static str] {
        &[] // No API key needed
    }

    fn is_configured(&self) -> bool {
        true // No API key needed — local scraper
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
