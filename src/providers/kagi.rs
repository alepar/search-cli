use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::{SearchOpts, SearchResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const SEARCH_URL: &str = "https://kagi.com/api/v1/search";
const EXTRACT_URL: &str = "https://kagi.com/api/v1/extract";
/// Logged per Extract call, and the name `stats` prices it under: pages are
/// billed separately from searches.
pub const EXTRACT_SOURCE: &str = "kagi_extract";
/// The API clamps `timeout` to 0.5-10 s.
const EXTRACT_TIMEOUT_SECS: u64 = 10;

pub struct Kagi {
    ctx: Arc<AppContext>,
}

impl Kagi {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }

    fn api_key(&self) -> String {
        super::resolve_key(&self.ctx.config.keys.kagi, "KAGI_API_KEY")
    }

    async fn do_search(
        &self,
        query: &str,
        count: usize,
        opts: &SearchOpts,
        workflow: Workflow,
    ) -> Result<Vec<SearchResult>, SearchError> {
        let key = self.api_key();
        if key.is_empty() {
            return Err(SearchError::AuthMissing { provider: "kagi" });
        }

        let body = build_request(query, count, opts, workflow);
        let client = &self.ctx.client;
        let resp = super::retry_request(|| async {
            let r = client
                .post(SEARCH_URL)
                .bearer_auth(&key)
                .json(&body)
                .send()
                .await?;
            let r = super::ok_or_api_error(r, "kagi")
                .await
                .map_err(normalize_error)?;
            Ok(r.json::<KagiResponse>().await?)
        })
        .await?;

        Ok(map_results(resp, workflow))
    }

    /// One page as Markdown through Kagi Extract, a live fetch rather than a
    /// cache. No retry: the caller decides what a bad page is worth. A block
    /// page can come back as a 200 with Markdown, so check what this returns.
    pub async fn extract_page(&self, url: &str, deadline: Duration) -> Result<String, SearchError> {
        let key = self.api_key();
        if key.is_empty() {
            return Err(SearchError::AuthMissing { provider: "kagi" });
        }
        let body = json!({
            "pages": [{"url": url}],
            "timeout": EXTRACT_TIMEOUT_SECS,
            "format": "json",
        });
        let resp = self
            .ctx
            .client
            .post(EXTRACT_URL)
            .bearer_auth(&key)
            .timeout(deadline)
            .json(&body)
            .send()
            .await?;
        let resp = super::ok_or_api_error(resp, "kagi")
            .await
            .map_err(normalize_error)?;
        extract_markdown(resp.json().await?)
    }
}

/// The page's Markdown. Per-page failures arrive in a 200 next to an empty
/// `data`, under `errors`; the spec's error envelope calls the field `error`.
fn extract_markdown(resp: ExtractResponse) -> Result<String, SearchError> {
    let page = resp
        .data
        .into_iter()
        .flatten()
        .find_map(|p| p.markdown.filter(|m| !m.trim().is_empty()));
    if let Some(markdown) = page {
        return Ok(markdown);
    }
    let detail: Vec<String> = resp
        .errors
        .into_iter()
        .flatten()
        .map(|e| match e.message {
            Some(message) => format!("{}: {message}", e.code),
            None => e.code,
        })
        .collect();
    Err(SearchError::Api {
        provider: "kagi",
        code: "api_error",
        message: if detail.is_empty() {
            "Extract returned no page".to_string()
        } else {
            format!("Extract failed: {}", detail.join("; "))
        },
        status: None,
    })
}

#[derive(Deserialize)]
struct ExtractResponse {
    data: Option<Vec<ExtractPage>>,
    #[serde(alias = "error")]
    errors: Option<Vec<ExtractError>>,
}

#[derive(Deserialize)]
struct ExtractPage {
    markdown: Option<String>,
}

#[derive(Deserialize)]
struct ExtractError {
    #[serde(default)]
    code: String,
    message: Option<String>,
}

/// Which Kagi v1 workflow to run. Each workflow returns its results under a
/// different category key of `data`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Workflow {
    Search,
    News,
}

impl Workflow {
    fn as_str(self) -> &'static str {
        match self {
            Workflow::Search => "search",
            Workflow::News => "news",
        }
    }
}

/// Build the `POST /api/v1/search` body. Domain filters go in an inline lens;
/// freshness and country go in `filters`, which Kagi applies over any
/// overlapping lens option.
fn build_request(query: &str, count: usize, opts: &SearchOpts, workflow: Workflow) -> Value {
    let mut body = json!({
        "query": query,
        "workflow": workflow.as_str(),
        // `limit` only trims the response (billing is per request), but it
        // keeps the payload to what the caller asked for.
        "limit": count.clamp(1, 1024),
    });

    let mut lens = serde_json::Map::new();
    if !opts.include_domains.is_empty() {
        lens.insert("sites_included".into(), json!(opts.include_domains));
    }
    if !opts.exclude_domains.is_empty() {
        lens.insert("sites_excluded".into(), json!(opts.exclude_domains));
    }
    if !lens.is_empty() {
        body["lens"] = Value::Object(lens);
    }

    let mut filters = serde_json::Map::new();
    if let Some(days) = opts.freshness.as_deref().and_then(super::freshness_days) {
        filters.insert("after".into(), json!(super::date_days_ago(days)));
    }
    if let Some(country) = opts.country.as_deref().filter(|c| !c.is_empty()) {
        filters.insert("region".into(), json!(country.to_ascii_lowercase()));
    }
    if !filters.is_empty() {
        body["filters"] = Value::Object(filters);
    }

    body
}

/// Kagi answers a malformed Bearer token with HTTP 400
/// `general.invalid_token` rather than 401. Re-tag it as 401 so it is
/// categorized as an auth failure instead of a generic API error.
fn normalize_error(err: SearchError) -> SearchError {
    match err {
        SearchError::Api {
            provider,
            code,
            message,
            status: Some(400),
        } if message.contains("general.invalid_token") => SearchError::Api {
            provider,
            code,
            message,
            status: Some(401),
        },
        other => other,
    }
}

fn map_results(resp: KagiResponse, workflow: Workflow) -> Vec<SearchResult> {
    let data = resp.data.unwrap_or_default();
    let items = match workflow {
        Workflow::Search => data.search,
        Workflow::News => data.news,
    };
    items
        .unwrap_or_default()
        .into_iter()
        .filter(|r| !r.url.is_empty())
        .map(|r| SearchResult {
            title: r.title,
            url: r.url,
            snippet: r.snippet.unwrap_or_default(),
            source: "kagi".to_string(),
            published: r.time,
            image_url: r.image.and_then(|i| i.url),
            extra: None,
        })
        .collect()
}

#[derive(Deserialize)]
struct KagiResponse {
    data: Option<KagiData>,
}

/// `data` is an object keyed by result category. Only the categories this
/// provider maps are deserialized; the rest (related_search, infobox, ...)
/// are ignored.
#[derive(Deserialize, Default)]
struct KagiData {
    search: Option<Vec<KagiResult>>,
    news: Option<Vec<KagiResult>>,
}

#[derive(Deserialize)]
struct KagiResult {
    #[serde(default)]
    url: String,
    #[serde(default)]
    title: String,
    snippet: Option<String>,
    time: Option<String>,
    image: Option<KagiImage>,
}

#[derive(Deserialize)]
struct KagiImage {
    url: Option<String>,
}

#[async_trait]
impl super::Provider for Kagi {
    fn name(&self) -> &'static str {
        "kagi"
    }

    fn capabilities(&self) -> &[&'static str] {
        &["general", "news", "deep"]
    }

    fn env_keys(&self) -> &[&'static str] {
        &["KAGI_API_KEY", "SEARCH_KEYS_KAGI"]
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
        self.do_search(query, count, opts, Workflow::Search).await
    }

    async fn search_news(
        &self,
        query: &str,
        count: usize,
        opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        self.do_search(query, count, opts, Workflow::News).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::FailureCategory;

    fn parse(json: &str) -> KagiResponse {
        serde_json::from_str(json).expect("fixture parses")
    }

    // Shape per the v1 OpenAPI spec: `data` keyed by category, each item
    // with url/title required and snippet/time/image/props optional.
    const SEARCH_FIXTURE: &str = r#"{
        "meta": {"trace": "abc", "node": "us-east", "ms": 412},
        "data": {
            "search": [
                {
                    "url": "https://www.rust-lang.org/",
                    "title": "Rust Programming Language",
                    "snippet": "A language empowering everyone.",
                    "time": "2026-09-01T00:00:00Z",
                    "image": {"url": "https://kagi.com/proxy/rust.png", "width": 64, "height": 64},
                    "props": {"language": "en", "paywalled": false}
                },
                {"url": "https://doc.rust-lang.org/book/", "title": "The Rust Book"},
                {"url": "", "title": "dropped: no url"}
            ],
            "related_search": [{"url": "https://kagi.com/search?q=rust+book", "title": "rust book"}],
            "adjacent_question": [{"url": "https://example.com/q", "title": "Q", "props": {"question": "Is Rust fast?"}}],
            "news": [{"url": "https://news.example.com/rust", "title": "Rust 2.0"}]
        }
    }"#;

    #[test]
    fn maps_search_category_only() {
        let results = map_results(parse(SEARCH_FIXTURE), Workflow::Search);
        assert_eq!(
            results.len(),
            2,
            "empty-url item dropped, other categories ignored"
        );

        let first = &results[0];
        assert_eq!(first.url, "https://www.rust-lang.org/");
        assert_eq!(first.title, "Rust Programming Language");
        assert_eq!(first.snippet, "A language empowering everyone.");
        assert_eq!(first.source, "kagi");
        assert_eq!(first.published.as_deref(), Some("2026-09-01T00:00:00Z"));
        assert_eq!(
            first.image_url.as_deref(),
            Some("https://kagi.com/proxy/rust.png")
        );

        let second = &results[1];
        assert_eq!(second.snippet, "");
        assert!(second.published.is_none());
        assert!(second.image_url.is_none());
    }

    #[test]
    fn news_workflow_maps_news_category() {
        let results = map_results(parse(SEARCH_FIXTURE), Workflow::News);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url, "https://news.example.com/rust");
    }

    #[test]
    fn missing_or_null_data_yields_no_results() {
        assert!(map_results(parse(r#"{"meta": {}, "data": null}"#), Workflow::Search).is_empty());
        assert!(map_results(parse(r#"{"meta": {}, "data": {}}"#), Workflow::Search).is_empty());
    }

    #[test]
    fn error_body_parses_as_empty_response() {
        // Live v1 error bodies use `errors` (the spec says `error`); either
        // way `data` is null. Errors are caught by status before parsing, but
        // the body must not break deserialization if one slips through.
        let live = r#"{"meta": {"trace": "t"}, "data": null,
            "errors": [{"code": "general.unauthorized", "url": "https://kagi.com/api#todo", "message": "Unauthorized"}]}"#;
        assert!(map_results(parse(live), Workflow::Search).is_empty());
    }

    #[test]
    fn request_minimal() {
        let body = build_request("rust", 5, &SearchOpts::default(), Workflow::Search);
        assert_eq!(
            body,
            json!({"query": "rust", "workflow": "search", "limit": 5})
        );
    }

    #[test]
    fn request_with_filters() {
        let opts = SearchOpts {
            include_domains: vec!["rust-lang.org".into()],
            exclude_domains: vec!["reddit.com".into()],
            freshness: Some("week".into()),
            country: Some("GB".into()),
            lang: Some("de".into()),
        };
        let body = build_request("rust", 10, &opts, Workflow::News);
        assert_eq!(body["workflow"], "news");
        assert_eq!(body["lens"]["sites_included"], json!(["rust-lang.org"]));
        assert_eq!(body["lens"]["sites_excluded"], json!(["reddit.com"]));
        assert_eq!(
            body["filters"]["after"],
            json!(super::super::date_days_ago(7))
        );
        assert_eq!(body["filters"]["region"], "gb");
        assert!(body.get("lang").is_none(), "Kagi v1 has no language param");
    }

    #[test]
    fn request_limit_clamped() {
        let body = build_request("q", 0, &SearchOpts::default(), Workflow::Search);
        assert_eq!(body["limit"], 1);
        let body = build_request("q", 5000, &SearchOpts::default(), Workflow::Search);
        assert_eq!(body["limit"], 1024);
    }

    fn extract(json: &str) -> Result<String, SearchError> {
        extract_markdown(serde_json::from_str(json).expect("fixture parses"))
    }

    #[test]
    fn extract_returns_the_page_markdown() {
        let ok = r##"{"meta": {"trace": "t", "ms": 4030},
            "data": [{"url": "https://example.com/", "markdown": "# Title\n\nText"}]}"##;
        assert_eq!(extract(ok).unwrap(), "# Title\n\nText");
    }

    #[test]
    fn extract_page_errors_keep_their_detail() {
        let per_page = r#"{"meta": {}, "data": [],
            "errors": [{"code": "extract.invalid_url", "url": "https://help.kagi.com/x",
                        "message": "URL must be a valid HTTPS URL", "location": "pages[0].url"}]}"#;
        let spec_shape = per_page.replace("\"errors\"", "\"error\"");
        for body in [per_page, &spec_shape] {
            let err = extract(body).unwrap_err().to_string();
            assert!(err.contains("extract.invalid_url"), "{err}");
            assert!(err.contains("valid HTTPS URL"), "{err}");
        }
    }

    #[test]
    fn extract_with_nothing_usable_is_an_error() {
        for body in [
            r#"{"meta": {}, "data": []}"#,
            r#"{"meta": {}, "data": null}"#,
            r#"{"meta": {}, "data": [{"url": "https://a.test/", "markdown": "  "}]}"#,
        ] {
            assert!(extract(body).is_err(), "{body}");
        }
    }

    fn api(status: u16, message: &str) -> SearchError {
        SearchError::Api {
            provider: "kagi",
            code: "api_error",
            message: message.to_string(),
            status: Some(status),
        }
    }

    #[test]
    fn invalid_token_400_is_auth() {
        let msg = r#"HTTP 400: {"meta":{},"data":null,"errors":[{"code":"general.invalid_token","message":"Invalid token"}]}"#;
        let err = normalize_error(api(400, msg));
        assert_eq!(err.http_status(), Some(401));
        assert_eq!(err.category(), FailureCategory::Auth);
    }

    #[test]
    fn other_400_unchanged() {
        let err = normalize_error(api(400, "HTTP 400: bad query"));
        assert_eq!(err.http_status(), Some(400));
    }
}
