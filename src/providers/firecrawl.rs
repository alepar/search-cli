use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::{SearchOpts, SearchResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// Firecrawl search documents limit 1..=100.
pub(super) const MAX_COUNT: usize = 100;

pub struct Firecrawl {
    ctx: Arc<AppContext>,
}

impl Firecrawl {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }

    fn api_key(&self) -> String {
        super::resolve_key(&self.ctx.config.keys.firecrawl, "FIRECRAWL_API_KEY")
    }
}

#[derive(Deserialize)]
struct FirecrawlScrapeResponse {
    data: Option<FirecrawlData>,
}

#[derive(Deserialize)]
struct FirecrawlData {
    markdown: Option<String>,
    metadata: Option<FirecrawlMetadata>,
}

#[derive(Deserialize)]
struct FirecrawlMetadata {
    title: Option<String>,
    #[serde(rename = "sourceURL")]
    source_url: Option<String>,
    /// Status the target site returned to Firecrawl's fetcher.
    #[serde(rename = "statusCode")]
    status_code: Option<u16>,
}

#[async_trait]
impl super::Provider for Firecrawl {
    fn name(&self) -> &'static str {
        "firecrawl"
    }

    fn env_keys(&self) -> &[&'static str] {
        &["FIRECRAWL_API_KEY", "SEARCH_KEYS_FIRECRAWL"]
    }
    fn capabilities(&self) -> &[&'static str] {
        &["general", "scrape", "extract"]
    }

    fn is_configured(&self) -> bool {
        !self.api_key().is_empty()
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    async fn search(
        &self,
        query: &str,
        count: usize,
        _opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        if self.api_key().is_empty() {
            return Err(SearchError::AuthMissing {
                provider: "firecrawl",
            });
        }
        // Firecrawl v2 search endpoint — web search + scrape combo
        let client = &self.ctx.client;
        let auth = format!("Bearer {}", self.api_key());
        let body = json!({
            "query": query,
            "limit": count.min(MAX_COUNT),
            "scrapeOptions": { "formats": ["markdown"] }
        });

        super::retry_request(|| async {
            let resp = client
                .post("https://api.firecrawl.dev/v2/search")
                .header("Authorization", &auth)
                .json(&body)
                .send()
                .await?;

            let resp = super::ok_or_api_error(resp, "firecrawl").await?;

            let val: serde_json::Value = resp.json().await?;
            let results = val
                .get("data")
                .and_then(|d| d.get("web"))
                .or_else(|| val.get("data"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|item| SearchResult {
                            title: item
                                .get("title")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            url: item
                                .get("url")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            snippet: item
                                .get("description")
                                .or_else(|| item.get("markdown"))
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            source: "firecrawl_search".to_string(),
                            published: None,
                            image_url: None,
                            extra: None,
                        })
                        .collect()
                })
                .unwrap_or_default();

            Ok(results)
        })
        .await
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

impl Firecrawl {
    pub async fn scrape_url(&self, url: &str) -> Result<Vec<SearchResult>, SearchError> {
        if self.api_key().is_empty() {
            return Err(SearchError::AuthMissing {
                provider: "firecrawl",
            });
        }

        let client = &self.ctx.client;
        let auth = format!("Bearer {}", self.api_key());
        let body = json!({
            "url": url,
            "formats": ["markdown"]
        });

        super::retry_request(|| async {
            let resp = client
                .post("https://api.firecrawl.dev/v2/scrape")
                .header("Authorization", &auth)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;

            if resp.status() == 429 {
                return Err(SearchError::RateLimited {
                    provider: "firecrawl",
                });
            }
            let resp = super::ok_or_api_error(resp, "firecrawl").await?;

            let body: FirecrawlScrapeResponse = resp.json().await?;
            if let Some(data) = body.data {
                let meta = data.metadata.unwrap_or(FirecrawlMetadata {
                    title: None,
                    source_url: None,
                    status_code: None,
                });
                if let Some(wall) = target_wall(meta.status_code) {
                    return Err(SearchError::Blocked {
                        provider: "firecrawl",
                        code: wall.code(),
                        message: format!(
                            "target site answered HTTP {} to Firecrawl",
                            meta.status_code.unwrap_or_default()
                        ),
                    });
                }
                Ok(vec![SearchResult {
                    title: meta.title.unwrap_or_default(),
                    url: meta.source_url.unwrap_or_else(|| url.to_string()),
                    snippet: data.markdown.unwrap_or_default(),
                    source: "firecrawl".to_string(),
                    published: None,
                    image_url: None,
                    extra: None,
                }])
            } else {
                Ok(vec![])
            }
        })
        .await
    }
}

/// The target's own status, as Firecrawl reports it. A 401/403/999 from the
/// site means a wall, whatever markdown Firecrawl rendered from the block page.
fn target_wall(status: Option<u16>) -> Option<crate::walls::Wall> {
    use crate::walls::Wall;
    match status? {
        401 | 999 => Some(Wall::Login),
        403 => Some(Wall::Bot),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::target_wall;
    use crate::walls::Wall;

    #[test]
    fn target_status_maps_to_walls() {
        assert_eq!(target_wall(Some(403)), Some(Wall::Bot));
        assert_eq!(target_wall(Some(999)), Some(Wall::Login));
        assert_eq!(target_wall(Some(200)), None);
        assert_eq!(target_wall(None), None);
    }
}
