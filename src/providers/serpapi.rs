use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::{SearchOpts, SearchResult};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

/// Per-request `num` maximum: Google Scholar pages hold 20, web search 100.
pub(super) fn max_count(engine: &str) -> usize {
    if engine == "google_scholar" {
        20
    } else {
        100
    }
}

pub struct SerpApi {
    ctx: Arc<AppContext>,
}

impl SerpApi {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }

    fn api_key(&self) -> String {
        super::resolve_key(&self.ctx.config.keys.serpapi, "SERPAPI_API_KEY")
    }

    pub fn is_configured(&self) -> bool {
        !self.api_key().is_empty()
    }

    async fn query_endpoint(
        &self,
        engine: &str,
        query: &str,
        count: usize,
        opts: &SearchOpts,
    ) -> Result<serde_json::Value, SearchError> {
        if self.api_key().is_empty() {
            return Err(SearchError::AuthMissing {
                provider: "serpapi",
            });
        }

        let news = engine == "google_news";
        let q = augment_query(query, opts, news);
        let client = &self.ctx.client;
        let api_key = self.api_key().to_string();

        let mut params = vec![
            ("engine".to_string(), engine.to_string()),
            ("q".to_string(), q),
            ("api_key".to_string(), api_key),
        ];
        // The Google News engine takes neither `num` nor `tbs`; freshness goes
        // into the query as a `when:` operator instead (augment_query).
        if !news {
            params.push(("num".to_string(), count.min(max_count(engine)).to_string()));
        }

        if let Some(f) = opts.freshness.as_ref().filter(|_| !news) {
            let tbs = match f.as_str() {
                "day" => "qdr:d",
                "week" => "qdr:w",
                "month" => "qdr:m",
                "year" => "qdr:y",
                other => other,
            };
            params.push(("tbs".to_string(), tbs.to_string()));
        }
        if let Some(country) = &opts.country {
            params.push(("gl".to_string(), country.to_lowercase()));
        }
        if let Some(lang) = &opts.lang {
            params.push(("hl".to_string(), lang.to_lowercase()));
        }

        super::retry_request(|| async {
            let resp = client
                .get("https://serpapi.com/search")
                .query(&params)
                .send()
                .await?;

            let resp = super::ok_or_api_error(resp, "serpapi").await?;

            Ok(resp.json().await?)
        })
        .await
    }
}

fn augment_query(query: &str, opts: &SearchOpts, news: bool) -> String {
    let mut q = super::with_site_operators(query, opts);
    if let Some(when) = opts
        .freshness
        .as_deref()
        .and_then(news_when)
        .filter(|_| news)
    {
        q = format!("{q} when:{when}");
    }
    q
}

/// Google News `when:` operator for a freshness window.
fn news_when(freshness: &str) -> Option<&'static str> {
    match freshness {
        "day" => Some("1d"),
        "week" => Some("7d"),
        "month" => Some("30d"),
        "year" => Some("1y"),
        _ => None,
    }
}

fn parse_results(body: &serde_json::Value, key: &str, source: &str) -> Vec<SearchResult> {
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
                    let url = item
                        .get("link")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let snippet = item
                        .get("snippet")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    // Google News gives iso_date next to a display-format date.
                    let published = item
                        .get("iso_date")
                        .or_else(|| item.get("date"))
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let image_url = None;

                    // For scholar results, attach citation info as extra
                    let extra = if source == "serpapi_scholar" {
                        let mut map = serde_json::Map::new();
                        if let Some(pub_info) = item.get("publication_info") {
                            map.insert("publication_info".to_string(), pub_info.clone());
                        }
                        if let Some(cited) = item.pointer("/inline_links/cited_by") {
                            map.insert("cited_by".to_string(), cited.clone());
                        }
                        if map.is_empty() {
                            None
                        } else {
                            Some(serde_json::Value::Object(map))
                        }
                    } else {
                        None
                    };

                    SearchResult {
                        title,
                        url,
                        snippet,
                        source: source.to_string(),
                        published,
                        image_url,
                        extra,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

#[async_trait]
impl super::Provider for SerpApi {
    fn name(&self) -> &'static str {
        "serpapi"
    }
    fn env_keys(&self) -> &[&'static str] {
        &["SERPAPI_API_KEY", "SEARCH_KEYS_SERPAPI"]
    }
    fn capabilities(&self) -> &[&'static str] {
        &["general", "news", "scholar", "images"]
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
        let body = self.query_endpoint("google", query, count, opts).await?;
        Ok(parse_results(&body, "organic_results", "serpapi"))
    }

    async fn search_news(
        &self,
        query: &str,
        count: usize,
        opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let body = self
            .query_endpoint("google_news", query, count, opts)
            .await?;
        Ok(parse_results(&body, "news_results", "serpapi_news"))
    }
}

impl SerpApi {
    pub async fn search_scholar(
        &self,
        query: &str,
        count: usize,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let body = self
            .query_endpoint("google_scholar", query, count, &SearchOpts::default())
            .await?;
        Ok(parse_results(&body, "organic_results", "serpapi_scholar"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scholar_reads_cited_by_from_inline_links() {
        let body = json!({"organic_results": [{
            "title": "Attention Is All You Need",
            "link": "https://arxiv.org/abs/1706.03762",
            "snippet": "The dominant sequence transduction models...",
            "publication_info": {"summary": "A Vaswani - NeurIPS, 2017"},
            "inline_links": {"cited_by": {"total": 150000, "cites_id": "2960712678066186980"}}
        }]});
        let r = parse_results(&body, "organic_results", "serpapi_scholar");
        let extra = r[0].extra.as_ref().unwrap();
        assert_eq!(extra["cited_by"]["total"], 150000);
        assert_eq!(
            extra["publication_info"]["summary"],
            "A Vaswani - NeurIPS, 2017"
        );
    }

    #[test]
    fn news_prefers_iso_date() {
        let body = json!({"news_results": [{"title": "t", "link": "https://e.com",
            "date": "10/30/2026, 07:00 AM, +0000 UTC", "iso_date": "2026-10-30T07:00:00Z"}]});
        let r = parse_results(&body, "news_results", "serpapi_news");
        assert_eq!(r[0].published.as_deref(), Some("2026-10-30T07:00:00Z"));
    }

    #[test]
    fn news_freshness_becomes_when_operator() {
        let opts = SearchOpts {
            freshness: Some("week".into()),
            ..SearchOpts::default()
        };
        assert_eq!(augment_query("rust", &opts, true), "rust when:7d");
        assert_eq!(augment_query("rust", &opts, false), "rust");
    }
}
