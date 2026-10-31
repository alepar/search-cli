//! `search usage` — remaining credits/quota for every configured provider
//! whose API exposes a usage or billing endpoint. Purely informational: the
//! CLI never gates, deprioritizes, or disables a provider based on balance —
//! the caller decides what to top up. Providers without a usage API are
//! reported as `supported: false` so their absence is explicit, not silent.

use crate::context::AppContext;
use crate::errors::redact_secrets;
use crate::providers;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;
use tokio::time::timeout;

pub(crate) const USAGE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Serialize)]
pub struct ProviderUsage {
    pub provider: String,
    /// Whether this provider's API exposes a usage/credits endpoint at all.
    pub supported: bool,
    /// Whether the provider is configured locally (has a key).
    pub configured: bool,
    /// Provider-reported usage figures, normalized where possible. Keys vary
    /// by provider (each bills differently); `credits_remaining` is populated
    /// whenever the provider reports a spendable balance; `summary`, when
    /// present, is a one-line human description that replaces the balance
    /// (e.g. pay-as-you-go accounts with no finite balance).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn unsupported(name: &str, configured: bool) -> ProviderUsage {
    ProviderUsage {
        provider: name.to_string(),
        supported: false,
        configured,
        data: None,
        error: None,
    }
}

/// reqwest's Display echoes the request URL, and SerpApi's carries the key.
fn transport_error(e: reqwest::Error) -> String {
    redact_secrets(&e.to_string())
}

pub(crate) async fn get_json(
    ctx: &AppContext,
    url: &str,
    bearer: Option<&str>,
    header: Option<(&str, &str)>,
) -> Result<serde_json::Value, String> {
    let mut req = ctx.client.get(url);
    if let Some(token) = bearer {
        req = req.bearer_auth(token);
    }
    if let Some((k, v)) = header {
        req = req.header(k, v);
    }
    send_json(req).await
}

/// Send a request and parse a JSON body; non-2xx becomes an error carrying
/// the status and a short whitespace-collapsed excerpt of the body. All error
/// text is redacted.
pub(crate) async fn send_json(req: reqwest::RequestBuilder) -> Result<serde_json::Value, String> {
    let resp = req.send().await.map_err(transport_error)?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let excerpt: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
        let excerpt: String = excerpt.chars().take(160).collect();
        return Err(redact_secrets(&format!("HTTP {status}: {excerpt}")));
    }
    resp.json::<serde_json::Value>()
        .await
        .map_err(transport_error)
}

/// SerpApi: GET https://serpapi.com/account.json?api_key=...
async fn serpapi_usage(ctx: &AppContext, key: String) -> Result<serde_json::Value, String> {
    let url = format!("https://serpapi.com/account.json?api_key={key}");
    let v = get_json(ctx, &url, None, None).await?;
    Ok(serde_json::json!({
        "credits_remaining": v.get("total_searches_left").or(v.get("plan_searches_left")).cloned(),
        "plan_searches_left": v.get("plan_searches_left").cloned(),
        "this_month_usage": v.get("this_month_usage").cloned(),
        "searches_per_month": v.get("searches_per_month").cloned(),
        "unit": "searches",
    }))
}

/// Brave has no balance endpoint, but every search response carries
/// X-RateLimit-Remaining, whose second comma-separated value is the
/// remaining monthly quota. Reading it requires one minimal metered
/// request (~$0.005) — flagged in the output so the cost is explicit.
async fn brave_usage(ctx: &AppContext, key: String) -> Result<serde_json::Value, String> {
    let resp = ctx
        .client
        .get("https://api.search.brave.com/res/v1/web/search?q=a&count=1")
        .header("X-Subscription-Token", key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(transport_error)?;
    let remaining = resp
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let status = resp.status();
    if !status.is_success() {
        return Err(format!("HTTP {status}"));
    }
    let monthly = remaining
        .as_deref()
        .and_then(|s| s.split(',').nth(1))
        .and_then(|s| s.trim().parse::<u64>().ok());
    Ok(serde_json::json!({
        "credits_remaining": monthly,
        "raw_rate_limit_remaining": remaining,
        "unit": "requests (this month)",
        "note": "read from X-RateLimit-Remaining headers; this check consumed one metered request",
    }))
}

/// xAI: GET https://management-api.x.ai/v1/billing/teams/{team_id}/prepaid/balance.
/// Needs a management key (not the xai- inference key) plus a team id, so both
/// come from dedicated env vars.
async fn xai_usage(ctx: &AppContext, _key: String) -> Result<serde_json::Value, String> {
    let mgmt_key = std::env::var("XAI_MANAGEMENT_API_KEY").unwrap_or_default();
    let team_id = std::env::var("XAI_TEAM_ID").unwrap_or_default();
    if mgmt_key.is_empty() || team_id.is_empty() {
        return Err(
            "set XAI_MANAGEMENT_API_KEY and XAI_TEAM_ID (management key differs from the xai- inference key)"
                .to_string(),
        );
    }
    let url = format!("https://management-api.x.ai/v1/billing/teams/{team_id}/prepaid/balance");
    let v = get_json(ctx, &url, Some(&mgmt_key), None).await?;
    let cents = v.get("total").and_then(|t| t.get("val")).and_then(|x| {
        x.as_f64()
            .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
    });
    Ok(serde_json::json!({
        "credits_remaining": cents.map(|c| c / 100.0),
        "unit": "USD",
    }))
}

/// Firecrawl: GET https://api.firecrawl.dev/v2/team/credit-usage (Bearer).
/// Falls back to the v1 path if v2 is unavailable.
async fn firecrawl_usage(ctx: &AppContext, key: String) -> Result<serde_json::Value, String> {
    let v = match get_json(
        ctx,
        "https://api.firecrawl.dev/v2/team/credit-usage",
        Some(&key),
        None,
    )
    .await
    {
        Ok(v) => v,
        Err(_) => {
            get_json(
                ctx,
                "https://api.firecrawl.dev/v1/team/credit-usage",
                Some(&key),
                None,
            )
            .await?
        }
    };
    let data = v.get("data").unwrap_or(&v);
    Ok(serde_json::json!({
        "credits_remaining": data.get("remainingCredits").or(data.get("remaining_credits")).cloned(),
        "plan_credits": data.get("planCredits").or(data.get("plan_credits")).cloned(),
        "unit": "credits",
    }))
}

/// Tavily: GET https://api.tavily.com/usage (Bearer tvly- key).
async fn tavily_usage(ctx: &AppContext, key: String) -> Result<serde_json::Value, String> {
    let v = get_json(ctx, "https://api.tavily.com/usage", Some(&key), None).await?;
    Ok(tavily_usage_from(&v))
}

/// Tavily reports the plan's included credits (`plan_limit`) and everything
/// used this cycle (`plan_usage`). On pay-as-you-go, usage past the plan
/// is billed as overage (`paygo_usage`, capped by `paygo_limit` when one is
/// set), so `plan_limit - plan_usage` going negative means overage, not an
/// exhausted account.
fn tavily_usage_from(v: &serde_json::Value) -> serde_json::Value {
    let account = v.get("account").unwrap_or(v);
    let num = |k: &str| account.get(k).and_then(|x| x.as_f64());
    let plan_usage = num("plan_usage");
    let plan_limit = num("plan_limit");
    let paygo_usage = num("paygo_usage");
    let paygo_limit = num("paygo_limit");

    let plan_remaining = match (plan_usage, plan_limit) {
        (Some(u), Some(l)) => Some((l - u).max(0.0)),
        _ => None,
    };
    let on_paygo = paygo_usage.is_some_and(|u| u > 0.0)
        || matches!((plan_usage, plan_limit), (Some(u), Some(l)) if u > l);
    // Spendable balance: what is left of the plan, plus the PAYG headroom when
    // a cap is set. Uncapped PAYG has no finite balance, so it stays null.
    let credits_remaining = match (on_paygo, paygo_limit) {
        (false, _) => plan_remaining,
        (true, Some(cap)) => {
            Some(plan_remaining.unwrap_or(0.0) + (cap - paygo_usage.unwrap_or(0.0)).max(0.0))
        }
        (true, None) => None,
    };
    let fmt = |x: f64| format!("{x:.0}");
    let summary = on_paygo.then(|| {
        let cap = paygo_limit.map_or("no cap".to_string(), |c| format!("cap {}", fmt(c)));
        format!(
            "pay-as-you-go: {} credits of overage this cycle ({cap}); plan {}/{} used",
            fmt(paygo_usage.unwrap_or(0.0)),
            plan_usage.map_or("?".into(), fmt),
            plan_limit.map_or("?".into(), fmt),
        )
    });

    let mut out = serde_json::json!({
        "credits_remaining": credits_remaining,
        "plan_remaining": plan_remaining,
        "plan_usage": plan_usage,
        "plan_limit": plan_limit,
        "pay_as_you_go": on_paygo,
        "paygo_usage": paygo_usage,
        "paygo_limit": paygo_limit,
        "current_plan": account.get("current_plan").cloned(),
        "unit": "credits",
    });
    if let Some(summary) = summary {
        out["summary"] = summary.into();
    }
    out
}

/// Linkup: GET https://api.linkup.so/v1/credits/balance (Bearer) -> { balance }.
async fn linkup_usage(ctx: &AppContext, key: String) -> Result<serde_json::Value, String> {
    let v = get_json(
        ctx,
        "https://api.linkup.so/v1/credits/balance",
        Some(&key),
        None,
    )
    .await?;
    Ok(serde_json::json!({
        "credits_remaining": v.get("balance").cloned(),
        "unit": "credits (EUR)",
    }))
}

/// Serper: GET https://google.serper.dev/account (X-API-KEY) -> { balance, rateLimit }.
async fn serper_usage(ctx: &AppContext, key: String) -> Result<serde_json::Value, String> {
    let v = get_json(
        ctx,
        "https://google.serper.dev/account",
        None,
        Some(("X-API-KEY", &key)),
    )
    .await?;
    Ok(serper_usage_from(&v))
}

fn serper_usage_from(v: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "credits_remaining": v.get("balance").cloned(),
        "rate_limit": v.get("rateLimit").cloned(),
        "unit": "credits",
    })
}

/// Jina: the API dashboard's backend (embeddings-dashboard-api.jina.ai, not
/// in Jina's public docs) reports the key's token wallet. Only the balance
/// is kept; the response also carries account details we have no use for.
async fn jina_usage(ctx: &AppContext, key: String) -> Result<serde_json::Value, String> {
    let url = format!("{JINA_DASHBOARD}/api/v1/api_key/user?api_key={key}");
    let v = get_json(ctx, &url, None, None).await?;
    Ok(jina_usage_from(&v))
}

pub(crate) const JINA_DASHBOARD: &str = "https://embeddings-dashboard-api.jina.ai";

fn jina_usage_from(v: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "credits_remaining": v.get("wallet").and_then(|w| w.get("total_balance")).cloned(),
        "unit": "tokens",
    })
}

/// Fetch usage for every provider, concurrently. Providers without a usage
/// API come back `supported: false`; unconfigured ones are skipped for
/// network calls but still listed.
/// Confirmed no supported balance API (dashboard-only) as of 2026-10:
/// perplexity (no balance endpoint for API keys), browserless, and parallel
/// (its balance endpoint needs an OAuth account token, not the API key).
/// Kagi API v1 reports no balance in responses and has no balance endpoint.
/// Exa exposes only cost-over-time per key id behind a separate service key
/// (no remaining balance) — not wired up.
pub async fn collect(ctx: Arc<AppContext>) -> Vec<ProviderUsage> {
    type UsageFetch = fn(
        &AppContext,
        String,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<serde_json::Value, String>> + Send + '_>,
    >;

    // Providers with a usage endpoint. Extend here as APIs appear.
    fn fetcher(name: &str) -> Option<UsageFetch> {
        match name {
            "serpapi" => Some(|ctx, key| Box::pin(serpapi_usage(ctx, key))),
            "firecrawl" => Some(|ctx, key| Box::pin(firecrawl_usage(ctx, key))),
            "brave" => Some(|ctx, key| Box::pin(brave_usage(ctx, key))),
            "xai" => Some(|ctx, key| Box::pin(xai_usage(ctx, key))),
            "tavily" => Some(|ctx, key| Box::pin(tavily_usage(ctx, key))),
            "linkup" => Some(|ctx, key| Box::pin(linkup_usage(ctx, key))),
            "serper" => Some(|ctx, key| Box::pin(serper_usage(ctx, key))),
            "jina" => Some(|ctx, key| Box::pin(jina_usage(ctx, key))),
            _ => None,
        }
    }

    let all = providers::build_providers(&ctx);
    let mut out = Vec::new();
    let mut set: JoinSet<ProviderUsage> = JoinSet::new();

    for p in &all {
        let name = p.name();
        let configured = p.is_configured();
        match fetcher(name) {
            None => out.push(unsupported(name, configured)),
            Some(fetch) => {
                if !configured {
                    out.push(ProviderUsage {
                        provider: name.to_string(),
                        supported: true,
                        configured: false,
                        data: None,
                        error: None,
                    });
                    continue;
                }
                let key = resolve_provider_key(&ctx, name);
                let ctx2 = ctx.clone();
                let name = name.to_string();
                set.spawn(async move {
                    let result = timeout(USAGE_TIMEOUT, fetch(&ctx2, key)).await;
                    match result {
                        Ok(Ok(data)) => ProviderUsage {
                            provider: name,
                            supported: true,
                            configured: true,
                            data: Some(data),
                            error: None,
                        },
                        Ok(Err(e)) => ProviderUsage {
                            provider: name,
                            supported: true,
                            configured: true,
                            data: None,
                            error: Some(e),
                        },
                        Err(_) => ProviderUsage {
                            provider: name,
                            supported: true,
                            configured: true,
                            data: None,
                            error: Some("timed out".to_string()),
                        },
                    }
                });
            }
        }
    }

    while let Some(Ok(u)) = set.join_next().await {
        out.push(u);
    }
    out.sort_by(|a, b| a.provider.cmp(&b.provider));

    // Snapshot balances so `search stats` can report measured burn over time.
    let snapshot: serde_json::Map<String, serde_json::Value> = out
        .iter()
        .filter_map(|u| u.data.clone().map(|d| (u.provider.clone(), d)))
        .collect();
    if !snapshot.is_empty() {
        crate::logging::log_balances(&serde_json::Value::Object(snapshot));
    }

    out
}

/// Resolve the API key for a provider by name, mirroring each provider's own
/// env > config precedence.
pub(crate) fn resolve_provider_key(ctx: &AppContext, name: &str) -> String {
    let k = &ctx.config.keys;
    match name {
        "serpapi" => providers::resolve_key(&k.serpapi, "SERPAPI_API_KEY"),
        "firecrawl" => providers::resolve_key(&k.firecrawl, "FIRECRAWL_API_KEY"),
        "exa" => providers::resolve_key(&k.exa, "EXA_API_KEY"),
        "jina" => providers::resolve_key(&k.jina, "JINA_API_KEY"),
        "linkup" => providers::resolve_key(&k.linkup, "LINKUP_API_KEY"),
        "kagi" => providers::resolve_key(&k.kagi, "KAGI_API_KEY"),
        "tavily" => providers::resolve_key(&k.tavily, "TAVILY_API_KEY"),
        "brave" => providers::resolve_key(&k.brave, "BRAVE_API_KEY"),
        "serper" => providers::resolve_key(&k.serper, "SERPER_API_KEY"),
        "perplexity" => providers::resolve_key(&k.perplexity, "PERPLEXITY_API_KEY"),
        "browserless" => providers::resolve_key(&k.browserless, "BROWSERLESS_API_KEY"),
        "parallel" => providers::resolve_key(&k.parallel, "PARALLEL_API_KEY"),
        "xai" => providers::resolve_key(&k.xai, "XAI_API_KEY"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{jina_usage_from, serper_usage_from, tavily_usage_from};
    use serde_json::json;

    #[test]
    fn tavily_paygo_overage_is_not_a_negative_balance() {
        // Shape of a live /usage response on a PAYG account past its plan.
        let v = json!({"account": {"current_plan": "Researcher", "plan_usage": 6962,
            "plan_limit": 1000, "paygo_usage": 5948, "paygo_limit": null}});
        let u = tavily_usage_from(&v);
        assert_eq!(u["pay_as_you_go"], true);
        assert_eq!(u["plan_remaining"], 0.0);
        assert!(u["credits_remaining"].is_null());
        assert_eq!(
            u["summary"],
            "pay-as-you-go: 5948 credits of overage this cycle (no cap); plan 6962/1000 used"
        );
    }

    #[test]
    fn tavily_paygo_cap_adds_headroom() {
        let v = json!({"account": {"plan_usage": 1200, "plan_limit": 1000,
            "paygo_usage": 200, "paygo_limit": 500}});
        assert_eq!(tavily_usage_from(&v)["credits_remaining"], 300.0);
    }

    #[test]
    fn tavily_plan_only() {
        let v = json!({"account": {"plan_usage": 250, "plan_limit": 1000, "paygo_usage": 0}});
        let u = tavily_usage_from(&v);
        assert_eq!(u["pay_as_you_go"], false);
        assert_eq!(u["credits_remaining"], 750.0);
        assert!(u.get("summary").is_none());
    }

    #[test]
    fn serper_balance() {
        let u = serper_usage_from(&json!({"balance": 49957, "rateLimit": 50}));
        assert_eq!(u["credits_remaining"], 49957);
        assert_eq!(u["unit"], "credits");
    }

    #[test]
    fn jina_balance_keeps_only_the_wallet_total() {
        let v = json!({"user_id": "u", "email": "someone@example.com",
            "wallet": {"regular_balance": 747633454, "total_balance": 747633454}});
        let u = jina_usage_from(&v);
        assert_eq!(u["credits_remaining"], 747633454);
        assert_eq!(u["unit"], "tokens");
        assert!(u.get("email").is_none());
    }
}
