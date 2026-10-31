//! Perplexity via the Agent API (`POST /v1/agent`).
//!
//! Sonar `/chat/completions` reached end of support on 2026-09-27; requests
//! there are reformulated into Agent API calls with undocumented handling of
//! the old top-level filters. We call the Agent API directly with the `fast`
//! effort level, Perplexity's documented replacement for `sonar-pro`. It
//! returns an answer with inline `[n]` citations plus a `search_results`
//! output item holding the sources. Search filters live on the `web_search`
//! tool.
//!
//! [`Perplexity::search_index`] calls the separate Search API (`POST
//! /search`), which returns raw ranked pages with page excerpts and no model
//! run.

use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::{SearchOpts, SearchResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const ENDPOINT: &str = "https://api.perplexity.ai/v1/agent";
const EFFORT: &str = "fast";
const INDEX_ENDPOINT: &str = "https://api.perplexity.ai/search";
/// Source label (and metadata name) for Search API results.
pub const INDEX_SOURCE: &str = "perplexity_search";
/// `search_domain_filter` accepts at most 20 entries.
const MAX_DOMAINS: usize = 20;

pub struct Perplexity {
    ctx: Arc<AppContext>,
}

impl Perplexity {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }

    fn api_key(&self) -> String {
        super::resolve_key(&self.ctx.config.keys.perplexity, "PERPLEXITY_API_KEY")
    }

    async fn do_search(
        &self,
        query: &str,
        opts: &SearchOpts,
        recency: Option<&str>,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let key = self.api_key();
        if key.is_empty() {
            return Err(SearchError::AuthMissing {
                provider: "perplexity",
            });
        }

        let body = build_request(query, opts, recency);

        // Agent runs need a longer timeout than the global 10s client.
        let pplx_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(45))
            .build()
            .unwrap_or_else(|_| self.ctx.client.clone());
        let resp = super::retry_request(|| {
            let body = body.clone();
            let key = key.clone();
            let pplx_client = pplx_client.clone();
            async move {
                let r = pplx_client
                    .post(ENDPOINT)
                    .header("Authorization", format!("Bearer {key}"))
                    .json(&body)
                    .send()
                    .await?;
                let r = super::ok_or_api_error(r, "perplexity").await?;
                Ok(r.json::<AgentResponse>().await?)
            }
        })
        .await?;

        map_results(resp)
    }

    /// Perplexity Search API (`POST /search`): ranked pages from Perplexity's
    /// own index, each with query-relevant page text of up to
    /// `max_tokens_per_page`. No model runs, so snippets are page text rather
    /// than a synthesis. This is how the reddit mode and the reddit.com
    /// extract route read Reddit without requesting reddit.com.
    pub async fn search_index(
        &self,
        query: &str,
        max_results: usize,
        opts: &SearchOpts,
        max_tokens_per_page: u32,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let key = self.api_key();
        if key.is_empty() {
            return Err(SearchError::AuthMissing {
                provider: "perplexity",
            });
        }
        let body = build_index_request(query, max_results, opts, max_tokens_per_page);
        let client = &self.ctx.client;
        let resp = super::retry_request(|| async {
            let r = client
                .post(INDEX_ENDPOINT)
                .header("Authorization", format!("Bearer {key}"))
                .json(&body)
                .send()
                .await?;
            let r = super::ok_or_api_error(r, "perplexity").await?;
            Ok(r.json::<IndexResponse>().await?)
        })
        .await?;
        Ok(map_index_results(resp))
    }
}

/// Build the Agent API request. The `web_search` tool is only sent when a
/// filter or location is set; otherwise the effort level's own tool config
/// applies unchanged. An explicit tool without `search_type` still inherits
/// the effort level's Fast Search.
fn build_request(query: &str, opts: &SearchOpts, recency: Option<&str>) -> Value {
    let mut body = json!({
        "effort": EFFORT,
        "input": query,
    });

    let mut filters = serde_json::Map::new();

    let domains = domain_filter(opts);
    if !domains.is_empty() {
        filters.insert("search_domain_filter".into(), json!(domains));
    }

    // Explicit recency first (news default), then the CLI's -f word, which
    // uses the same vocabulary (day/week/month/year).
    if let Some(r) = recency.or(opts.freshness.as_deref()) {
        filters.insert("search_recency_filter".into(), json!(r));
    }

    let mut tool = serde_json::Map::new();
    if !filters.is_empty() {
        tool.insert("filters".into(), Value::Object(filters));
    }
    // user_location sits beside `filters`; country is ISO 3166-1 alpha-2.
    if let Some(c) = opts.country.as_deref().filter(|c| !c.is_empty()) {
        tool.insert(
            "user_location".into(),
            json!({ "country": c.to_ascii_uppercase() }),
        );
    }
    if !tool.is_empty() {
        tool.insert("type".into(), json!("web_search"));
        body["tools"] = json!([tool]);
    }
    body
}

/// `search_domain_filter` entries: an allowlist, or a denylist of `-domain`
/// entries, never both in one request. An allowlist already excludes
/// everything else, so it wins when both are given.
fn domain_filter(opts: &SearchOpts) -> Vec<String> {
    let domains: Vec<String> = if !opts.include_domains.is_empty() {
        opts.include_domains.clone()
    } else {
        opts.exclude_domains
            .iter()
            .map(|d| format!("-{d}"))
            .collect()
    };
    domains.into_iter().take(MAX_DOMAINS).collect()
}

/// Search API request body. `max_results` is clamped to the API's 1..=20.
fn build_index_request(
    query: &str,
    max_results: usize,
    opts: &SearchOpts,
    max_tokens_per_page: u32,
) -> Value {
    let mut body = json!({
        "query": query,
        "max_results": max_results.clamp(1, 20),
        "max_tokens_per_page": max_tokens_per_page,
    });
    let domains = domain_filter(opts);
    if !domains.is_empty() {
        body["search_domain_filter"] = json!(domains);
    }
    if let Some(r) = opts.freshness.as_deref() {
        body["search_recency_filter"] = json!(r);
    }
    if let Some(c) = opts.country.as_deref().filter(|c| !c.is_empty()) {
        body["country"] = json!(c.to_ascii_uppercase());
    }
    body
}

/// Search API results as web results. The snippet is the page excerpt;
/// `last_updated` (when Perplexity last indexed the page) goes in `extra`.
fn map_index_results(resp: IndexResponse) -> Vec<SearchResult> {
    resp.results
        .into_iter()
        .filter(|r| !r.url.is_empty())
        .map(|r| SearchResult {
            title: r.title,
            url: r.url,
            snippet: r.snippet,
            source: INDEX_SOURCE.to_string(),
            published: r.date,
            image_url: None,
            extra: r.last_updated.map(|d| json!({ "index_last_updated": d })),
        })
        .collect()
}

#[derive(Deserialize)]
struct IndexResponse {
    #[serde(default)]
    results: Vec<IndexResult>,
}

#[derive(Deserialize)]
struct IndexResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    snippet: String,
    date: Option<String>,
    last_updated: Option<String>,
}

/// Turn an Agent API response into one answer pseudo-result followed by the
/// cited sources. `[n]` markers in the answer refer to source `id`s.
fn map_results(resp: AgentResponse) -> Result<Vec<SearchResult>, SearchError> {
    if resp.status == "failed" {
        let message = resp
            .error
            .and_then(|e| e.message)
            .unwrap_or_else(|| "agent run failed".to_string());
        return Err(SearchError::Api {
            provider: "perplexity",
            code: "api_error",
            message,
            status: None,
        });
    }

    let source_label = format!("perplexity_{EFFORT}");
    let mut answer = String::new();
    let mut sources = Vec::new();

    for item in resp.output {
        match item {
            OutputItem::Message { content } => {
                for part in content.into_iter().filter(|p| p.kind == "output_text") {
                    if !answer.is_empty() {
                        answer.push_str("\n\n");
                    }
                    answer.push_str(&part.text);
                }
            }
            OutputItem::SearchResults { results } => {
                sources.extend(results.into_iter().filter(|r| !r.url.is_empty()).map(|r| {
                    SearchResult {
                        snippet: if r.snippet.is_empty() {
                            "[Search result]".to_string()
                        } else {
                            r.snippet
                        },
                        title: r.title,
                        url: r.url,
                        source: format!("{source_label}_result"),
                        published: r.date,
                        image_url: None,
                        extra: None,
                    }
                }));
            }
            OutputItem::Other => {}
        }
    }

    let mut results = Vec::with_capacity(sources.len() + 1);
    if !answer.trim().is_empty() {
        results.push(SearchResult {
            title: "AI Answer".to_string(),
            url: "perplexity://answer".to_string(),
            snippet: answer,
            source: source_label,
            published: None,
            image_url: None,
            extra: None,
        });
    }
    results.extend(sources);
    Ok(results)
}

#[derive(Deserialize)]
struct AgentResponse {
    #[serde(default)]
    status: String,
    #[serde(default)]
    output: Vec<OutputItem>,
    error: Option<AgentError>,
}

#[derive(Deserialize)]
struct AgentError {
    message: Option<String>,
}

/// Output items we read; tool traces (fetch_url_results, function_call, ...)
/// are ignored. Unknown types must not break parsing.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutputItem {
    Message {
        #[serde(default)]
        content: Vec<ContentPart>,
    },
    SearchResults {
        #[serde(default)]
        results: Vec<AgentSearchResult>,
    },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct ContentPart {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct AgentSearchResult {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    snippet: String,
    date: Option<String>,
}

#[async_trait]
impl super::Provider for Perplexity {
    fn name(&self) -> &'static str {
        "perplexity"
    }
    fn env_keys(&self) -> &[&'static str] {
        &["PERPLEXITY_API_KEY", "SEARCH_KEYS_PERPLEXITY"]
    }
    fn capabilities(&self) -> &[&'static str] {
        &["general", "news", "academic", "deep"]
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
        _count: usize,
        opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        self.do_search(query, opts, None).await
    }

    async fn search_news(
        &self,
        query: &str,
        _count: usize,
        opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        // Default to "day" recency for news if no freshness specified
        let recency = opts.freshness.as_deref().unwrap_or("day");
        self.do_search(query, opts, Some(recency)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> AgentResponse {
        serde_json::from_str(json).expect("fixture parses")
    }

    // Shape per the Agent API reference (ResponsesResponse) and the Sonar
    // migration guide's recorded response: search_results, a tool trace we
    // ignore, then the message.
    const FIXTURE: &str = r#"{
        "id": "resp_18690c07-7c56-48fa-85a3-fd22e34df694",
        "created_at": 1784292159,
        "model": "openai/gpt-6-luna",
        "effort": "fast@2026-10-16.0",
        "object": "response",
        "status": "completed",
        "output": [
            {
                "type": "search_results",
                "queries": ["latest developments AI agents"],
                "results": [
                    {"id": 1, "title": "State of AI Agents 2026", "url": "https://www.prosus.com/state-of-ai-agents",
                     "snippet": "From one-shot intelligence to autonomy.", "date": "2026-02-26",
                     "last_updated": "2026-07-12", "source": "web"},
                    {"id": 2, "title": "CUDA Guide", "url": "https://docs.nvidia.com/cuda/", "snippet": "",
                     "date": null, "last_updated": "2026-05-21"},
                    {"id": 3, "title": "no url", "url": "", "snippet": "dropped"}
                ]
            },
            {"type": "fetch_url_results", "contents": [{"url": "https://x.example", "text": "ignored"}]},
            {
                "id": "msg_525b46ea",
                "type": "message",
                "role": "assistant",
                "status": "completed",
                "content": [
                    {"type": "output_text", "text": "Agents gained autonomy[1].", "annotations": []}
                ]
            }
        ],
        "usage": {"input_tokens": 5120, "output_tokens": 210, "cost": {"total_cost": 0.0021}}
    }"#;

    #[test]
    fn maps_answer_then_sources() {
        let results = map_results(parse(FIXTURE)).unwrap();
        assert_eq!(
            results.len(),
            3,
            "answer + 2 sources; empty-url source dropped"
        );

        let answer = &results[0];
        assert_eq!(answer.url, "perplexity://answer");
        assert_eq!(answer.snippet, "Agents gained autonomy[1].");
        assert_eq!(answer.source, "perplexity_fast");

        let first = &results[1];
        assert_eq!(first.url, "https://www.prosus.com/state-of-ai-agents");
        assert_eq!(first.title, "State of AI Agents 2026");
        assert_eq!(first.snippet, "From one-shot intelligence to autonomy.");
        assert_eq!(first.published.as_deref(), Some("2026-02-26"));
        assert_eq!(first.source, "perplexity_fast_result");

        let second = &results[2];
        assert_eq!(second.snippet, "[Search result]");
        assert!(second.published.is_none());
    }

    #[test]
    fn blank_answer_is_not_emitted() {
        let json = r#"{"status": "completed", "output": [
            {"type": "message", "id": "m", "role": "assistant", "status": "completed",
             "content": [{"type": "output_text", "text": "  "}]}
        ]}"#;
        assert!(map_results(parse(json)).unwrap().is_empty());
    }

    #[test]
    fn failed_run_is_an_error() {
        let json = r#"{"status": "failed", "output": [],
            "error": {"code": "server_error", "message": "model overloaded", "type": "server"}}"#;
        let Err(err) = map_results(parse(json)) else {
            panic!("failed run must error");
        };
        assert!(err.to_string().contains("model overloaded"), "{err}");
    }

    #[test]
    fn incomplete_run_keeps_partial_output() {
        let json = r#"{"status": "incomplete", "output": [
            {"type": "message", "id": "m", "role": "assistant", "status": "incomplete",
             "content": [{"type": "output_text", "text": "Partial"}]}
        ]}"#;
        let results = map_results(parse(json)).unwrap();
        assert_eq!(results[0].snippet, "Partial");
    }

    #[test]
    fn request_without_filters_uses_effort_defaults() {
        let body = build_request("rust 2.0", &SearchOpts::default(), None);
        assert_eq!(body, json!({"effort": "fast", "input": "rust 2.0"}));
    }

    #[test]
    fn filters_and_location_go_on_web_search_tool() {
        let o = SearchOpts {
            exclude_domains: vec!["reddit.com".into(), "pinterest.com".into()],
            freshness: Some("week".into()),
            country: Some("gb".into()),
            ..Default::default()
        };
        let body = build_request("q", &o, None);
        assert_eq!(
            body["tools"],
            json!([{
                "type": "web_search",
                "filters": {
                    "search_domain_filter": ["-reddit.com", "-pinterest.com"],
                    "search_recency_filter": "week"
                },
                "user_location": {"country": "GB"}
            }])
        );
    }

    #[test]
    fn allowlist_wins_over_denylist_and_is_capped() {
        let o = SearchOpts {
            include_domains: (0..25).map(|i| format!("d{i}.com")).collect(),
            exclude_domains: vec!["reddit.com".into()],
            ..Default::default()
        };
        let body = build_request("q", &o, None);
        let domains = body["tools"][0]["filters"]["search_domain_filter"]
            .as_array()
            .unwrap();
        assert_eq!(domains.len(), MAX_DOMAINS);
        assert!(domains
            .iter()
            .all(|d| !d.as_str().unwrap().starts_with('-')));
    }

    #[test]
    fn explicit_recency_overrides_freshness() {
        let o = SearchOpts {
            freshness: Some("year".into()),
            ..Default::default()
        };
        let body = build_request("q", &o, Some("day"));
        assert_eq!(
            body["tools"][0]["filters"]["search_recency_filter"],
            json!("day")
        );
    }

    #[test]
    fn index_request_carries_filters_and_clamps_count() {
        let o = SearchOpts {
            include_domains: vec!["reddit.com".into()],
            freshness: Some("week".into()),
            country: Some("gb".into()),
            ..Default::default()
        };
        let body = build_index_request("tokio vs smol", 50, &o, 2048);
        assert_eq!(body["query"], json!("tokio vs smol"));
        assert_eq!(body["max_results"], json!(20));
        assert_eq!(body["max_tokens_per_page"], json!(2048));
        assert_eq!(body["search_domain_filter"], json!(["reddit.com"]));
        assert_eq!(body["search_recency_filter"], json!("week"));
        assert_eq!(body["country"], json!("GB"));
        let bare = build_index_request("q", 0, &SearchOpts::default(), 512);
        assert_eq!(bare["max_results"], json!(1));
        assert!(bare.get("search_domain_filter").is_none());
        assert!(bare.get("search_recency_filter").is_none());
    }

    // Shape recorded from a live /search call (2026-10-31), trimmed.
    #[test]
    fn index_results_map_excerpt_and_dates() {
        let resp: IndexResponse = serde_json::from_str(
            r#"{"id": "x", "results": [
                {"title": "PSA: async-std has been officially discontinued",
                 "url": "https://www.reddit.com/r/rust/comments/1jc6gis/psa/",
                 "snippet": "PSA\n**votes: 452**\n### Comments\nUse smol.",
                 "date": "2025-03-15", "last_updated": "2025-11-03"},
                {"title": "no url", "url": "", "snippet": "dropped"},
                {"title": "undated", "url": "https://a.example/", "snippet": "s"}
            ]}"#,
        )
        .unwrap();
        let results = map_index_results(resp);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].source, INDEX_SOURCE);
        assert_eq!(results[0].published.as_deref(), Some("2025-03-15"));
        assert!(results[0].snippet.contains("### Comments"));
        assert_eq!(
            results[0].extra,
            Some(json!({"index_last_updated": "2025-11-03"}))
        );
        assert!(results[1].published.is_none() && results[1].extra.is_none());
    }
}
