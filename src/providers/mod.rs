pub mod brave;
pub mod browserless;
pub mod exa;
pub mod firecrawl;
pub mod jina;
pub mod kagi;
pub mod linkup;
pub mod parallel;
pub mod perplexity;
pub mod reddit;
pub mod serpapi;
pub mod serper;
#[cfg(feature = "stealth")]
pub mod stealth;
pub mod tavily;
pub mod xai;

use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::{SearchOpts, SearchResult};
use async_trait::async_trait;
use backon::{ExponentialBuilder, Retryable};
use std::sync::Arc;
use std::time::Duration;

pub async fn retry_request<F, Fut, T>(f: F) -> Result<T, SearchError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, SearchError>>,
{
    // Retry only TRANSIENT failures — rate limits, timeouts, network errors,
    // and 5xx (SearchError::is_retryable). The old predicate matched only
    // Http(_) transport errors, so 429/503 (mapped to RateLimited/Api before
    // the predicate saw them) were never retried — backoff was effectively
    // dead for the cases it exists for. Permanent 4xx (401/402/400) are NOT
    // retried; that would just burn the budget.
    //
    // The caller's Provider::timeout() bounds the whole retry sequence, so a
    // slow attempt can use the full budget and leave no room for a retry.
    // The client has a connect timeout only, so a transport Timeout here is
    // a connect failure, which is cheap to retry.
    f.retry(
        ExponentialBuilder::default()
            .with_jitter()
            .with_min_delay(Duration::from_millis(200))
            .with_max_delay(Duration::from_millis(800))
            .with_max_times(2),
    )
    .when(|e| e.is_retryable())
    .await
}

/// Resolve a provider API key. **Environment variable wins over the config
/// file** — CI/session credentials and a freshly-exported key must be able to
/// override stale local config. (Previously config won, so an out-of-date
/// `keys.brave` silently shadowed a working `BRAVE_API_KEY` and every brave
/// request failed.) Because figment already merges `SEARCH_KEYS_*` env over
/// TOML into `config_value`, the full precedence is:
/// bare env var (e.g. `BRAVE_API_KEY`) > `SEARCH_KEYS_*` env > config file.
pub fn resolve_key(config_value: &str, env_var: &str) -> String {
    pick_key(config_value, std::env::var(env_var).ok())
}

/// The precedence rule of [`resolve_key`], on an already-read env value.
fn pick_key(config_value: &str, env_value: Option<String>) -> String {
    env_value
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| config_value.to_string())
}

/// Turn a non-2xx reqwest response into a categorized [`SearchError`], preserving
/// a short, sanitized body excerpt so "HTTP 422" becomes
/// "HTTP 422: The provided subscription token is invalid." Returns the response
/// unchanged on 2xx. Centralizing this here keeps status/body/429 handling
/// identical across every provider (previously copy-pasted and drift-prone).
pub async fn ok_or_api_error(
    resp: reqwest::Response,
    provider: &'static str,
) -> Result<reqwest::Response, SearchError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let code = status.as_u16();
    if code == 429 {
        // Honor Retry-After (seconds form) up to 2s before surfacing the
        // error, so the backon retry that follows lands after the window the
        // provider asked for instead of blind-hammering a throttled API.
        // Capped so it always fits inside the provider timeout budget.
        let wait = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|s| s.min(2));
        if let Some(secs) = wait {
            tokio::time::sleep(Duration::from_secs(secs)).await;
        }
        return Err(SearchError::RateLimited { provider });
    }
    // Body is only read on the error path; the success path stays zero-copy.
    let body = resp.text().await.unwrap_or_default();
    Err(SearchError::Api {
        provider,
        code: "api_error",
        message: http_error_message(code, &body),
        status: Some(code),
    })
}

/// Format `HTTP <status>` with a sanitized one-line body excerpt when present.
pub fn http_error_message(status: u16, body: &str) -> String {
    let excerpt = sanitize_excerpt(body, 200);
    if excerpt.is_empty() {
        format!("HTTP {status}")
    } else {
        format!("HTTP {status}: {excerpt}")
    }
}

/// UTC date `days` ago as `YYYY-MM-DD`, without a date-crate dependency.
/// Used to translate the CLI's freshness words into provider date floors.
/// Append Google-style site operators for -d / --exclude-domain. Several
/// includes are OR-ed in one group: bare `site:a site:b` is an AND that no
/// page can satisfy. Exclusions stay `-site:` conjunctions.
pub fn with_site_operators(query: &str, opts: &SearchOpts) -> String {
    let mut q = query.to_string();
    match opts.include_domains.as_slice() {
        [] => {}
        [d] => q = format!("{q} site:{d}"),
        many => {
            let any = many
                .iter()
                .map(|d| format!("site:{d}"))
                .collect::<Vec<_>>()
                .join(" OR ");
            q = format!("{q} ({any})");
        }
    }
    for d in &opts.exclude_domains {
        q = format!("{q} -site:{d}");
    }
    q
}

pub fn date_days_ago(days: u64) -> String {
    crate::dates::epoch_days_to_date(crate::dates::today_epoch_days().saturating_sub(days))
}

/// Days of lookback for a freshness word (day/week/month/year).
pub fn freshness_days(freshness: &str) -> Option<u64> {
    match freshness {
        "day" => Some(1),
        "week" => Some(7),
        "month" => Some(30),
        "year" => Some(365),
        _ => None,
    }
}

/// Collapse whitespace and clip to `max` chars (char-safe, never panics on UTF-8).
fn sanitize_excerpt(body: &str, max: usize) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = collapsed.chars().take(max).collect();
    if collapsed.chars().count() > max {
        out.push('…');
    }
    out
}

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> &[&'static str];
    fn is_configured(&self) -> bool;
    /// Standard env var names accepted by this provider (e.g. BRAVE_API_KEY).
    fn env_keys(&self) -> &[&'static str];
    fn timeout(&self) -> Duration {
        Duration::from_secs(10)
    }

    async fn search(
        &self,
        query: &str,
        count: usize,
        opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError>;
    async fn search_news(
        &self,
        query: &str,
        count: usize,
        opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError>;
}

/// Last-resort HTML-to-text for scrapers when readability finds no article:
/// drops tags and the contents of <script>/<style>, collapses whitespace.
/// Works on chars, so multi-byte text survives intact.
pub fn html_to_text(html: &str) -> String {
    fn starts_with_ci(s: &str, prefix: &str) -> bool {
        s.as_bytes()
            .get(..prefix.len())
            .is_some_and(|b| b.eq_ignore_ascii_case(prefix.as_bytes()))
    }

    let mut text = String::with_capacity(html.len() / 3);
    let mut in_tag = false;
    let mut skip_until: Option<&str> = None;
    for (i, c) in html.char_indices() {
        if c == '<' {
            let rest = &html[i..];
            match skip_until {
                Some(end) if starts_with_ci(rest, end) => skip_until = None,
                None if starts_with_ci(rest, "<script") => skip_until = Some("</script"),
                None if starts_with_ci(rest, "<style") => skip_until = Some("</style"),
                _ => {}
            }
            in_tag = true;
            // Tags separate words: "<p>a</p><p>b</p>" reads "a b", not "ab".
            text.push(' ');
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag && skip_until.is_none() {
            text.push(c);
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn build_providers(ctx: &Arc<AppContext>) -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(parallel::Parallel::new(ctx.clone())),
        Box::new(brave::Brave::new(ctx.clone())),
        Box::new(serper::Serper::new(ctx.clone())),
        Box::new(exa::Exa::new(ctx.clone())),
        Box::new(jina::Jina::new(ctx.clone())),
        Box::new(linkup::Linkup::new(ctx.clone())),
        Box::new(kagi::Kagi::new(ctx.clone())),
        #[cfg(feature = "stealth")]
        Box::new(stealth::Stealth::new(ctx.clone())),
        Box::new(firecrawl::Firecrawl::new(ctx.clone())),
        Box::new(tavily::Tavily::new(ctx.clone())),
        Box::new(browserless::Browserless::new(ctx.clone())),
        Box::new(perplexity::Perplexity::new(ctx.clone())),
        Box::new(serpapi::SerpApi::new(ctx.clone())),
        Box::new(xai::Xai::new(ctx.clone())),
    ]
}

#[cfg(test)]
mod site_operator_tests {
    use super::*;

    fn opts(inc: &[&str], exc: &[&str]) -> SearchOpts {
        SearchOpts {
            include_domains: inc.iter().map(|s| s.to_string()).collect(),
            exclude_domains: exc.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn several_include_domains_are_or_ed() {
        assert_eq!(with_site_operators("q", &opts(&[], &[])), "q");
        assert_eq!(
            with_site_operators("q", &opts(&["a.com"], &[])),
            "q site:a.com"
        );
        assert_eq!(
            with_site_operators("q", &opts(&["a.com", "b.com"], &["c.com"])),
            "q (site:a.com OR site:b.com) -site:c.com"
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn per_request_counts_stay_within_documented_maxima() {
        // `-c 30` (recommended for deep mode) used to fail brave web with
        // 422 and jina with 400; fusion handles the total, each request
        // must stay within the provider's cap.
        assert_eq!(super::brave::WEB_MAX_COUNT, 20);
        assert_eq!(super::brave::NEWS_MAX_COUNT, 50);
        assert_eq!(super::brave::LLM_CONTEXT_MAX_COUNT, 50);
        assert_eq!(super::jina::MAX_COUNT, 20);
        assert_eq!(super::serper::MAX_COUNT, 10);
        assert_eq!(super::serpapi::max_count("google_scholar"), 20);
        assert_eq!(super::serpapi::max_count("google"), 100);
        assert_eq!(super::exa::MAX_COUNT, 100);
        assert_eq!(super::firecrawl::MAX_COUNT, 100);
        assert_eq!(super::parallel::MAX_COUNT, 40);
    }

    #[test]
    fn html_to_text_strips_tags_scripts_and_styles() {
        let html = "<html><head><style>p{color:red}</style><SCRIPT>var a = '<b>';</SCRIPT></head>\
                    <body><p>Hello</p><p>world</p></body></html>";
        assert_eq!(super::html_to_text(html), "Hello world");
    }

    #[test]
    fn html_to_text_keeps_multibyte_text_after_angle_bracket() {
        // A multi-byte char right after '<' used to panic on a byte-index
        // slice; non-ASCII text also came out as Latin-1 mojibake.
        assert_eq!(
            super::html_to_text("<é> naïve 日本語 <p>ok"),
            "naïve 日本語 ok"
        );
        assert_eq!(super::html_to_text("<"), "");
    }

    use super::pick_key;

    // Tests the precedence on values, not the process environment: setting
    // env vars is unsafe in edition 2024 while other test threads run.
    #[test]
    fn env_var_wins_over_config() {
        assert_eq!(pick_key("from-config", Some("from-env".into())), "from-env");
    }

    #[test]
    fn falls_back_to_config_when_env_unset() {
        assert_eq!(pick_key("from-config", None), "from-config");
    }

    #[test]
    fn empty_env_does_not_shadow_config() {
        assert_eq!(pick_key("from-config", Some(String::new())), "from-config");
    }

    #[test]
    fn empty_when_neither_set() {
        assert_eq!(pick_key("", None), "");
    }
}
