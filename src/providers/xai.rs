use crate::config::AppConfig;
use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::{SearchOpts, SearchResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// Default model for X search. grok-4-1-fast was retired 2026-05-15 and its
/// slug now redirects here; grok-4.3 is also the cheapest current tier with
/// server-side tool support. Override with `xai.model` / `SEARCH_XAI_MODEL`.
pub const DEFAULT_MODEL: &str = "grok-4.3";

/// x_search accepts at most this many handles in either filter list.
const MAX_X_HANDLES: usize = 20;

/// The model to call: `[xai] model` from config, else [`DEFAULT_MODEL`].
pub fn model(config: &AppConfig) -> &str {
    let m = config.xai.model.trim();
    if m.is_empty() {
        DEFAULT_MODEL
    } else {
        m
    }
}

pub struct Xai {
    ctx: Arc<AppContext>,
}

impl Xai {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }

    fn api_key(&self) -> String {
        super::resolve_key(&self.ctx.config.keys.xai, "XAI_API_KEY")
    }

    async fn call_responses_api(
        &self,
        prompt: &str,
        x_search_config: Option<serde_json::Value>,
    ) -> Result<XaiResponse, SearchError> {
        let api_key = self.api_key();
        if api_key.is_empty() {
            return Err(SearchError::AuthMissing { provider: "xai" });
        }

        let mut x_search_tool = json!({"type": "x_search"});
        if let Some(config) = x_search_config {
            if let (Some(tool_obj), Some(config_obj)) =
                (x_search_tool.as_object_mut(), config.as_object())
            {
                for (k, v) in config_obj {
                    tool_obj.insert(k.clone(), v.clone());
                }
            }
        }

        let body = json!({
            "model": model(&self.ctx.config),
            "input": [{"role": "user", "content": prompt}],
            "tools": [x_search_tool],
            "store": false,
        });

        let client = &self.ctx.client;
        let key = api_key;

        // xAI needs its own client with a longer timeout — the global client
        // has a 10s timeout which is too short for LLM inference + tool use
        let xai_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(90))
            .build()
            .unwrap_or_else(|_| client.clone());

        super::retry_request(|| {
            let body = body.clone();
            let key = key.clone();
            let xai_client = xai_client.clone();
            async move {
                let resp = xai_client
                    .post("https://api.x.ai/v1/responses")
                    .header("Authorization", format!("Bearer {key}"))
                    .header("Content-Type", "application/json")
                    .json(&body)
                    .send()
                    .await?;

                let resp = super::ok_or_api_error(resp, "xai").await?;

                let body_bytes = resp.bytes().await?;
                let mut body_vec = body_bytes.to_vec();
                simd_json::from_slice(&mut body_vec).map_err(|e| SearchError::Api {
                    provider: "xai",
                    code: "json_error",
                    status: None,
                    message: e.to_string(),
                })
            }
        })
        .await
    }
}

fn build_x_search_config(opts: &SearchOpts) -> Option<serde_json::Value> {
    let mut config = serde_json::Map::new();

    // -d/--exclude-domain carry X handles here. x_search rejects requests
    // that set both lists, so an allow-list wins: it already excludes every
    // handle not on it.
    let allowed = x_handles(&opts.include_domains);
    let excluded = x_handles(&opts.exclude_domains);
    if !allowed.is_empty() {
        config.insert("allowed_x_handles".to_string(), json!(allowed));
    } else if !excluded.is_empty() {
        config.insert("excluded_x_handles".to_string(), json!(excluded));
    }

    if let Some(days_back) = opts.freshness.as_deref().and_then(super::freshness_days) {
        let (from, to) = date_range(crate::dates::today_epoch_days(), days_back);
        config.insert("from_date".to_string(), json!(from));
        config.insert("to_date".to_string(), json!(to));
    }

    if config.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(config))
    }
}

/// Handles go to x_search without the leading `@`, deduplicated and capped
/// at the API maximum.
fn x_handles(raw: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    raw.iter()
        .map(|h| h.trim().trim_start_matches('@').to_string())
        .filter(|h| !h.is_empty() && seen.insert(h.to_ascii_lowercase()))
        .take(MAX_X_HANDLES)
        .collect()
}

/// x_search dates are UTC `YYYY-MM-DD` and `to_date` is exclusive, so the
/// range ends tomorrow — ending it today would drop today's posts.
fn date_range(today: u64, days_back: u64) -> (String, String) {
    (
        crate::dates::epoch_days_to_date(today.saturating_sub(days_back)),
        crate::dates::epoch_days_to_date(today + 1),
    )
}

#[derive(Deserialize)]
struct XaiResponse {
    output: Option<Vec<XaiOutputItem>>,
}

#[derive(Deserialize)]
struct XaiOutputItem {
    #[serde(rename = "type")]
    item_type: Option<String>,
    content: Option<Vec<XaiContent>>,
    text: Option<String>,
}

#[derive(Deserialize)]
struct XaiContent {
    #[serde(rename = "type")]
    content_type: Option<String>,
    text: Option<String>,
    url: Option<String>,
    annotations: Option<Vec<XaiAnnotation>>,
}

#[derive(Deserialize)]
struct XaiAnnotation {
    #[serde(rename = "type")]
    annotation_type: Option<String>,
    url: Option<String>,
}

fn extract_text(resp: &XaiResponse) -> String {
    let mut parts = Vec::new();
    if let Some(output) = &resp.output {
        for item in output {
            if item.item_type.as_deref() == Some("message") {
                if let Some(content) = &item.content {
                    for c in content {
                        if c.content_type.as_deref() == Some("output_text") {
                            if let Some(text) = &c.text {
                                parts.push(text.clone());
                            }
                        }
                    }
                }
            } else if let Some(text) = &item.text {
                parts.push(text.clone());
            }
        }
    }
    parts.join("\n")
}

fn extract_citations(resp: &XaiResponse) -> Vec<String> {
    let mut urls = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some(output) = &resp.output {
        for item in output {
            if let Some(content) = &item.content {
                for c in content {
                    // Extract from url_citation annotations (primary method)
                    if let Some(annotations) = &c.annotations {
                        for ann in annotations {
                            if ann.annotation_type.as_deref() == Some("url_citation") {
                                if let Some(url) = &ann.url {
                                    if seen.insert(url.clone()) {
                                        urls.push(url.clone());
                                    }
                                }
                            }
                        }
                    }
                    // Fallback: check for cite/url content types
                    if c.content_type.as_deref() == Some("cite")
                        || c.content_type.as_deref() == Some("url")
                    {
                        if let Some(url) = &c.url {
                            if seen.insert(url.clone()) {
                                urls.push(url.clone());
                            }
                        }
                    }
                }
            }
        }
    }
    urls
}

#[async_trait]
impl super::Provider for Xai {
    fn name(&self) -> &'static str {
        "xai"
    }
    fn env_keys(&self) -> &[&'static str] {
        &["XAI_API_KEY", "SEARCH_KEYS_XAI"]
    }
    fn capabilities(&self) -> &[&'static str] {
        &["social"]
    }
    fn is_configured(&self) -> bool {
        !self.api_key().is_empty()
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(60)
    }

    async fn search(
        &self,
        query: &str,
        count: usize,
        opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let prompt = format!(
            "Search X (Twitter) for posts about: {query}\n\
             Return up to {count} recent and relevant posts.\n\
             For each post include: author @username, post text, date/time, \
             and engagement metrics (likes, reposts, replies) if available.\n\
             Format as a clear summary."
        );

        let x_config = build_x_search_config(opts);
        let resp = self.call_responses_api(&prompt, x_config).await?;

        let mut results = Vec::new();
        let answer = extract_text(&resp);

        if !answer.is_empty() {
            results.push(SearchResult {
                title: "X Summary".to_string(),
                url: format!("https://x.com/search?q={}", query.replace(' ', "+")),
                snippet: answer,
                source: "xai".to_string(),
                published: None,
                image_url: None,
                extra: None,
            });
        }

        // Append citations as separate results
        let citations = extract_citations(&resp);
        for cite_url in citations {
            let hostname = url::Url::parse(&cite_url)
                .ok()
                .and_then(|u| u.path().split('/').nth(1).map(|s| format!("@{s}")))
                .unwrap_or_else(|| cite_url.clone());

            results.push(SearchResult {
                title: hostname,
                url: cite_url,
                snippet: "[Citation from X]".to_string(),
                source: "xai_citation".to_string(),
                published: None,
                image_url: None,
                extra: None,
            });
        }

        Ok(results)
    }

    async fn search_news(
        &self,
        query: &str,
        count: usize,
        opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        // X is inherently news-like; delegate to search
        self.search(query, count, opts).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> XaiResponse {
        // Same parser as the live path.
        let mut bytes = json.as_bytes().to_vec();
        simd_json::from_slice(&mut bytes).expect("fixture parses")
    }

    fn opts(include: &[&str], exclude: &[&str], freshness: Option<&str>) -> SearchOpts {
        SearchOpts {
            include_domains: include.iter().map(|s| s.to_string()).collect(),
            exclude_domains: exclude.iter().map(|s| s.to_string()).collect(),
            freshness: freshness.map(str::to_string),
            ..Default::default()
        }
    }

    // Shape per the xAI Responses API docs (tools/x-search, tools/citations):
    // reasoning and custom_tool_call items precede the message; inline
    // citations arrive as url_citation annotations on output_text, with the
    // visible citation number in `title`; a top-level `citations` list holds
    // every source seen, cited or not.
    const RESPONSE_FIXTURE: &str = r#"{
        "id": "resp_123",
        "object": "response",
        "model": "grok-4.3",
        "status": "completed",
        "output": [
            {"type": "reasoning", "id": "rs_1", "summary": []},
            {
                "type": "custom_tool_call",
                "id": "ctc_1",
                "call_id": "call_1",
                "name": "x_keyword_search",
                "input": "{\"query\":\"rust 2.0\",\"limit\":10}",
                "status": "completed"
            },
            {
                "type": "message",
                "id": "msg_1",
                "role": "assistant",
                "status": "completed",
                "content": [
                    {
                        "type": "output_text",
                        "text": "@rustlang announced it [[1]](https://x.com/rustlang/status/1). Others agreed [[2]](https://x.com/jdoe/status/2) [[1]](https://x.com/rustlang/status/1).",
                        "logprobs": [],
                        "annotations": [
                            {"type": "url_citation", "url": "https://x.com/rustlang/status/1", "start_index": 23, "end_index": 59, "title": "1"},
                            {"type": "url_citation", "url": "https://x.com/jdoe/status/2", "start_index": 76, "end_index": 108, "title": "2"},
                            {"type": "url_citation", "url": "https://x.com/rustlang/status/1", "start_index": 109, "end_index": 145, "title": "1"}
                        ]
                    }
                ]
            }
        ],
        "citations": [
            "https://x.com/rustlang/status/1",
            "https://x.com/jdoe/status/2",
            "https://x.com/unrelated/status/3"
        ],
        "usage": {
            "input_tokens": 5120,
            "output_tokens": 410,
            "server_side_tool_usage_details": {"x_search_calls": 1, "x_posts_fetched": 10, "x_users_fetched": 0}
        }
    }"#;

    #[test]
    fn extracts_message_text_only() {
        let text = extract_text(&parse(RESPONSE_FIXTURE));
        assert!(text.starts_with("@rustlang announced it"));
        assert!(!text.contains("x_keyword_search"));
    }

    #[test]
    fn citations_come_from_annotations_deduplicated_in_order() {
        assert_eq!(
            extract_citations(&parse(RESPONSE_FIXTURE)),
            vec![
                "https://x.com/rustlang/status/1",
                "https://x.com/jdoe/status/2"
            ]
        );
    }

    #[test]
    fn empty_and_minimal_responses_parse() {
        let resp = parse(r#"{"output": []}"#);
        assert!(extract_text(&resp).is_empty());
        assert!(extract_citations(&resp).is_empty());
        let resp = parse(r#"{"id": "resp_1"}"#);
        assert!(extract_text(&resp).is_empty());
    }

    #[test]
    fn no_filters_means_no_tool_config() {
        assert!(build_x_search_config(&opts(&[], &[], None)).is_none());
    }

    #[test]
    fn handle_lists_are_mutually_exclusive() {
        let cfg = build_x_search_config(&opts(&["rustlang"], &["spam"], None)).unwrap();
        assert_eq!(cfg["allowed_x_handles"], json!(["rustlang"]));
        assert!(cfg.get("excluded_x_handles").is_none());

        let cfg = build_x_search_config(&opts(&[], &["spam"], None)).unwrap();
        assert_eq!(cfg["excluded_x_handles"], json!(["spam"]));
        assert!(cfg.get("allowed_x_handles").is_none());
    }

    #[test]
    fn handles_are_normalized_and_capped() {
        let raw: Vec<String> = (0..25).map(|i| format!("@user{i}")).collect();
        let handles = x_handles(&raw);
        assert_eq!(handles.len(), MAX_X_HANDLES);
        assert_eq!(handles[0], "user0");

        let raw = vec![
            " @RustLang ".to_string(),
            "rustlang".to_string(),
            "@".to_string(),
            "jdoe".to_string(),
        ];
        assert_eq!(x_handles(&raw), vec!["RustLang", "jdoe"]);
    }

    #[test]
    fn date_range_end_is_exclusive_tomorrow() {
        // 2026-10-31 is epoch day 20757.
        assert_eq!(crate::dates::epoch_days_to_date(20757), "2026-10-31");
        assert_eq!(
            date_range(20757, 1),
            ("2026-10-30".to_string(), "2026-11-01".to_string())
        );
        assert_eq!(
            date_range(20757, 7),
            ("2026-10-24".to_string(), "2026-11-01".to_string())
        );
    }

    #[test]
    fn freshness_maps_to_date_window() {
        let cfg = build_x_search_config(&opts(&[], &[], Some("week"))).unwrap();
        assert!(cfg["from_date"].is_string());
        assert!(cfg["to_date"].is_string());
        assert!(build_x_search_config(&opts(&[], &[], Some("bogus"))).is_none());
    }

    #[test]
    fn model_defaults_and_overrides() {
        let mut config = AppConfig::default();
        assert_eq!(model(&config), DEFAULT_MODEL);
        config.xai.model = "  ".to_string();
        assert_eq!(model(&config), DEFAULT_MODEL);
        config.xai.model = "grok-4.7".to_string();
        assert_eq!(model(&config), "grok-4.7");
    }
}
