use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::{SearchOpts, SearchResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// Serper returns one Google page (10 results) per request. A larger `num`
/// is echoed but ignored on web search, and on news num=30 returns an empty
/// list with no error (checked live 2026-10-31).
pub(super) const MAX_COUNT: usize = 10;

pub struct Serper {
    ctx: Arc<AppContext>,
}

impl Serper {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }

    fn api_key(&self) -> String {
        super::resolve_key(&self.ctx.config.keys.serper, "SERPER_API_KEY")
    }

    async fn query_endpoint(
        &self,
        endpoint: &str,
        query: &str,
        count: usize,
        opts: &SearchOpts,
    ) -> Result<serde_json::Value, SearchError> {
        if self.api_key().is_empty() {
            return Err(SearchError::AuthMissing { provider: "serper" });
        }

        // Serper uses site: in query for domain filtering
        let q = augment_query(query, opts);
        let url = format!("https://google.serper.dev/{endpoint}");
        let client = &self.ctx.client;
        let api_key = self.api_key();

        let mut body = json!({ "q": q, "num": count.min(MAX_COUNT) });
        // Serper freshness via tbs param: qdr:d, qdr:w, qdr:m, qdr:y
        if let Some(f) = &opts.freshness {
            let tbs = match f.as_str() {
                "day" => "qdr:d",
                "week" => "qdr:w",
                "month" => "qdr:m",
                "year" => "qdr:y",
                other => other,
            };
            body["tbs"] = json!(tbs);
        }
        if let Some(country) = &opts.country {
            body["gl"] = json!(country.to_lowercase());
        }
        if let Some(lang) = &opts.lang {
            body["hl"] = json!(lang.to_lowercase());
        }

        super::retry_request(|| async {
            let resp = client
                .post(&url)
                .header("X-API-KEY", api_key.as_str())
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;

            let resp = super::ok_or_api_error(resp, "serper").await?;

            let body_bytes = resp.bytes().await?;
            let mut body_vec = body_bytes.to_vec();
            simd_json::from_slice(&mut body_vec).map_err(|e| SearchError::Api {
                provider: "serper",
                code: "json_error",
                status: None,
                message: e.to_string(),
            })
        })
        .await
    }
}

fn augment_query(query: &str, opts: &SearchOpts) -> String {
    super::with_site_operators(query, opts)
}

fn parse_organic(body: &serde_json::Value, source: &str) -> Vec<SearchResult> {
    let key = match source {
        "serper_news" => "news",
        "serper_images" => "images",
        "serper_places" => "places",
        "serper_scholar" | "serper_patents" => "organic",
        _ => "organic",
    };

    let items = body.get(key).and_then(|v| v.as_array());
    items
        .map(|arr| {
            arr.iter()
                .map(|item| {
                    let title = item
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let str_field = |k: &str| item.get(k).and_then(|v| v.as_str());
                    // Places carry `website` (often absent) and a `cid`
                    // instead of `link`, and an address instead of a snippet.
                    let url = str_field("link")
                        .or_else(|| str_field("website"))
                        .map(String::from)
                        .or_else(|| {
                            // cid may come back as a string or a number.
                            item.get("cid")
                                .filter(|v| v.is_string() || v.is_u64())
                                .map(|cid| {
                                    let cid =
                                        cid.as_str().map_or_else(|| cid.to_string(), String::from);
                                    format!("https://maps.google.com/?cid={cid}")
                                })
                        })
                        .unwrap_or_default();
                    let snippet = str_field("snippet")
                        .or_else(|| str_field("address"))
                        .unwrap_or_default()
                        .to_string();
                    let published = item.get("date").and_then(|v| v.as_str()).map(String::from);
                    let image_url = item
                        .get("imageUrl")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    SearchResult {
                        title,
                        url,
                        snippet,
                        source: source.to_string(),
                        published,
                        image_url,
                        extra: None,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

#[async_trait]
impl super::Provider for Serper {
    fn name(&self) -> &'static str {
        "serper"
    }
    fn capabilities(&self) -> &[&'static str] {
        &["general", "news", "scholar", "patents", "images", "places"]
    }
    fn env_keys(&self) -> &[&'static str] {
        &["SERPER_API_KEY", "SEARCH_KEYS_SERPER"]
    }
    fn is_configured(&self) -> bool {
        !self.api_key().is_empty()
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }

    async fn search(
        &self,
        query: &str,
        count: usize,
        opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let body = self.query_endpoint("search", query, count, opts).await?;
        Ok(parse_organic(&body, "serper"))
    }

    async fn search_news(
        &self,
        query: &str,
        count: usize,
        opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let body = self.query_endpoint("news", query, count, opts).await?;
        Ok(parse_organic(&body, "serper_news"))
    }
}

impl Serper {
    pub async fn search_scholar(
        &self,
        query: &str,
        count: usize,
    ) -> Result<Vec<SearchResult>, SearchError> {
        self.search_special("scholar", query, count).await
    }
    pub async fn search_patents(
        &self,
        query: &str,
        count: usize,
    ) -> Result<Vec<SearchResult>, SearchError> {
        self.search_special("patents", query, count).await
    }
    pub async fn search_images(
        &self,
        query: &str,
        count: usize,
    ) -> Result<Vec<SearchResult>, SearchError> {
        self.search_special("images", query, count).await
    }
    pub async fn search_places(
        &self,
        query: &str,
        count: usize,
    ) -> Result<Vec<SearchResult>, SearchError> {
        self.search_special("places", query, count).await
    }

    async fn search_special(
        &self,
        endpoint: &str,
        query: &str,
        count: usize,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let body = self
            .query_endpoint(endpoint, query, count, &SearchOpts::default())
            .await?;
        Ok(parse_organic(&body, &format!("serper_{endpoint}")))
    }
}

#[cfg(test)]
mod tests {
    use super::parse_organic;
    use serde_json::json;

    #[test]
    fn organic_results() {
        let body = json!({"organic": [{"title": "Rust", "link": "https://rust-lang.org",
            "snippet": "A language", "date": "Oct 30, 2026"}]});
        let r = parse_organic(&body, "serper");
        assert_eq!(
            (r[0].url.as_str(), r[0].snippet.as_str()),
            ("https://rust-lang.org", "A language")
        );
        assert_eq!(r[0].published.as_deref(), Some("Oct 30, 2026"));
    }

    #[test]
    fn news_and_images_use_their_own_keys() {
        let body = json!({"news": [{"title": "n", "link": "https://n.com", "imageUrl": "https://n.com/i.jpg"}],
            "images": [{"title": "i", "link": "https://i.com", "imageUrl": "https://i.com/x.png"}]});
        assert_eq!(
            parse_organic(&body, "serper_news")[0].image_url.as_deref(),
            Some("https://n.com/i.jpg")
        );
        assert_eq!(
            parse_organic(&body, "serper_images")[0].url,
            "https://i.com"
        );
    }

    #[test]
    fn places_fall_back_to_website_then_maps_cid() {
        let body = json!({"places": [
            {"title": "Cafe", "address": "1 Main St", "website": "https://cafe.example", "cid": "123"},
            {"title": "Bar", "address": "2 Main St", "cid": "456"}
        ]});
        let r = parse_organic(&body, "serper_places");
        assert_eq!(
            (r[0].url.as_str(), r[0].snippet.as_str()),
            ("https://cafe.example", "1 Main St")
        );
        assert_eq!(r[1].url, "https://maps.google.com/?cid=456");
    }
}
