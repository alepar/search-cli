//! `search billing` — monthly usage per provider over the last six UTC
//! calendar months, newest first. Every cell records where its number came
//! from, so sources are never mixed silently:
//!
//! - `api`: the provider's own usage history (jina, firecrawl, xai), in the
//!   provider's native unit (tokens, credits, USD).
//! - `api_current_period`: the provider reports only its current billing
//!   cycle (tavily, serpapi); shown in the current-month column only.
//! - `estimate`: USD estimated from the local search logs with the
//!   `search stats` price table, for months the logs cover.
//! - `none`: no data (before the local logs start).
//!
//! Values are never converted between units.

use crate::context::AppContext;
use crate::usage::{get_json, resolve_provider_key, send_json, JINA_DASHBOARD, USAGE_TIMEOUT};
use crate::{dates, logging, providers, stats};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::task::JoinSet;
use tokio::time::timeout;

pub const MONTHS: usize = 6;

/// Jina history is paged newest first; 1,000 records per page covers weeks
/// of hourly aggregates, and the page cap bounds a runaway walk.
const JINA_PAGE: usize = 1000;
const JINA_MAX_PAGES: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Api,
    ApiCurrentPeriod,
    Estimate,
    None,
}

#[derive(Debug, Serialize)]
pub struct Cell {
    /// `YYYY-MM`, UTC.
    pub month: String,
    pub value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    pub source: Source,
}

#[derive(Debug, Serialize)]
pub struct ProviderBilling {
    pub provider: String,
    pub configured: bool,
    /// Unit of the provider-reported figures, when its API reports any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Newest month first, aligned with `BillingReport::months`.
    pub months: Vec<Cell>,
    /// Why the provider's usage API could not be read; cells fall back to
    /// local-log estimates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BillingReport {
    /// Newest first.
    pub months: Vec<String>,
    /// Month of the oldest local search log; earlier months have no estimate.
    pub estimates_since: Option<String>,
    pub providers: Vec<ProviderBilling>,
}

/// What a provider's usage API reports.
#[derive(Debug, PartialEq)]
enum History {
    Monthly {
        unit: &'static str,
        by_month: BTreeMap<String, f64>,
    },
    CurrentPeriod {
        unit: &'static str,
        value: f64,
    },
}

/// The `n` calendar months ending with the one containing `today` (days
/// since the epoch), newest first, as `YYYY-MM`.
fn last_months(today: u64, n: usize) -> Vec<String> {
    let (mut y, mut m) = year_month(&dates::epoch_days_to_date(today));
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(format!("{y:04}-{m:02}"));
        (y, m) = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
    }
    out
}

/// `(year, month)` of a `YYYY-MM[...]` string.
fn year_month(s: &str) -> (i64, u32) {
    let y = s.get(0..4).and_then(|x| x.parse().ok()).unwrap_or(1970);
    let m = s.get(5..7).and_then(|x| x.parse().ok()).unwrap_or(1);
    (y, m)
}

fn next_month(month: &str) -> String {
    let (y, m) = year_month(month);
    let (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    format!("{y:04}-{m:02}")
}

/// `YYYY-MM` prefix of an ISO-8601 timestamp, if it has one.
fn month_of(ts: &str) -> Option<&str> {
    let m = ts.get(0..7)?;
    (m.as_bytes()[4] == b'-').then_some(m)
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

// ---- provider history parsers -------------------------------------------

/// Add one page of Jina transactions (newest first) to `by_month`, counting
/// `consume` records only. Returns true when paging should stop: the page is
/// short, or it reached records older than `oldest`.
fn jina_accumulate(page: &Value, oldest: &str, by_month: &mut BTreeMap<String, f64>) -> bool {
    let Some(rows) = page.as_array() else {
        return true;
    };
    let mut past = false;
    for r in rows {
        let when = r
            .pointer("/metadata/time_period")
            .and_then(Value::as_str)
            .or_else(|| r.get("created_at").and_then(Value::as_str))
            .unwrap_or("");
        let Some(month) = month_of(when) else {
            continue;
        };
        if month < oldest {
            past = true;
            continue;
        }
        if r.get("type").and_then(Value::as_str) != Some("consume") {
            continue;
        }
        *by_month.entry(month.to_string()).or_default() +=
            r.get("quantity").and_then(num).unwrap_or(0.0);
    }
    past || rows.len() < JINA_PAGE
}

/// Firecrawl: `periods[]` are billing periods; each is credited to the month
/// it starts in.
fn firecrawl_history_from(v: &Value) -> History {
    let mut by_month = BTreeMap::new();
    for p in v
        .get("periods")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(month) = p
            .get("startDate")
            .and_then(Value::as_str)
            .and_then(month_of)
        else {
            continue;
        };
        *by_month.entry(month.to_string()).or_default() +=
            p.get("totalCredits").and_then(num).unwrap_or(0.0);
    }
    History::Monthly {
        unit: "credits",
        by_month,
    }
}

/// xAI management API usage query: USD summed per calendar month (UTC) over
/// `[start, end)`, both `YYYY-MM`.
fn xai_usage_request(start: &str, end: &str) -> Value {
    serde_json::json!({
        "analyticsRequest": {
            "timeRange": {
                "startTime": format!("{start}-01 00:00:00"),
                "endTime": format!("{end}-01 00:00:00"),
                "timezone": "Etc/UTC",
            },
            "timeUnit": "TIME_UNIT_MONTH",
            "values": [{"name": "usd", "aggregation": "AGGREGATION_SUM"}],
            "groupBy": [],
            "filters": [],
        }
    })
}

/// Sum every series' data points per month (a grouped response has one
/// series per group).
fn xai_history_from(v: &Value) -> History {
    let mut by_month = BTreeMap::new();
    for series in v
        .get("timeSeries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for dp in series
            .get("dataPoints")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(month) = dp
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(month_of)
            else {
                continue;
            };
            let value = dp
                .get("values")
                .and_then(|x| x.get(0))
                .and_then(num)
                .unwrap_or(0.0);
            *by_month.entry(month.to_string()).or_default() += value;
        }
    }
    History::Monthly {
        unit: "USD",
        by_month,
    }
}

/// Tavily: credits used so far this billing cycle (plan plus overage).
fn tavily_history_from(v: &Value) -> Result<History, String> {
    let account = v.get("account").unwrap_or(v);
    let value = account
        .get("plan_usage")
        .and_then(num)
        .ok_or("no plan_usage in /usage response")?;
    Ok(History::CurrentPeriod {
        unit: "credits",
        value,
    })
}

/// SerpApi: searches used so far this month.
fn serpapi_history_from(v: &Value) -> Result<History, String> {
    let value = v
        .get("this_month_usage")
        .and_then(num)
        .ok_or("no this_month_usage in account.json")?;
    Ok(History::CurrentPeriod {
        unit: "searches",
        value,
    })
}

// ---- fetchers ------------------------------------------------------------

fn has_history(provider: &str) -> bool {
    matches!(
        provider,
        "jina" | "firecrawl" | "xai" | "tavily" | "serpapi"
    )
}

async fn fetch_history(
    ctx: &AppContext,
    provider: &str,
    key: &str,
    months: &[String],
) -> Result<History, String> {
    let oldest = months.last().map(String::as_str).unwrap_or_default();
    match provider {
        "jina" => {
            let mut by_month = BTreeMap::new();
            for page in 0..JINA_MAX_PAGES {
                let url = format!(
                    "{JINA_DASHBOARD}/api/v1/billing/paginated_transaction?num_records={JINA_PAGE}&offset={}",
                    page * JINA_PAGE
                );
                let v = get_json(ctx, &url, Some(key), None).await?;
                if jina_accumulate(&v, oldest, &mut by_month) {
                    break;
                }
            }
            Ok(History::Monthly {
                unit: "tokens",
                by_month,
            })
        }
        "firecrawl" => get_json(
            ctx,
            "https://api.firecrawl.dev/v2/team/credit-usage/historical?byApiKey=false",
            Some(key),
            None,
        )
        .await
        .map(|v| firecrawl_history_from(&v)),
        "xai" => {
            let mgmt_key = std::env::var("XAI_MANAGEMENT_API_KEY").unwrap_or_default();
            let team_id = std::env::var("XAI_TEAM_ID").unwrap_or_default();
            if mgmt_key.is_empty() || team_id.is_empty() {
                return Err(
                    "set XAI_MANAGEMENT_API_KEY and XAI_TEAM_ID (management key differs from the xai- inference key)"
                        .to_string(),
                );
            }
            let newest = months.first().map(String::as_str).unwrap_or_default();
            let url = format!("https://management-api.x.ai/v1/billing/teams/{team_id}/usage");
            let req = ctx
                .client
                .post(url)
                .bearer_auth(mgmt_key)
                .json(&xai_usage_request(oldest, &next_month(newest)));
            send_json(req).await.map(|v| xai_history_from(&v))
        }
        "tavily" => {
            let v = get_json(ctx, "https://api.tavily.com/usage", Some(key), None).await?;
            tavily_history_from(&v)
        }
        "serpapi" => {
            let url = format!("https://serpapi.com/account.json?api_key={key}");
            let v = get_json(ctx, &url, None, None).await?;
            serpapi_history_from(&v)
        }
        _ => Err("no usage history API".to_string()),
    }
}

// ---- local-log estimates -------------------------------------------------

/// Estimated USD per base provider per month from search-log entries. Same
/// rule as `search stats`: only fresh (uncached) runs are billed.
fn estimates_by_month(entries: &[Value]) -> BTreeMap<String, BTreeMap<String, f64>> {
    let mut out: BTreeMap<String, BTreeMap<String, f64>> = BTreeMap::new();
    for e in entries {
        if e.get("cached").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let Some(ts) = e.get("ts").and_then(Value::as_u64) else {
            continue;
        };
        let date = dates::epoch_days_to_date(ts / 86400);
        let month = date[0..7].to_string();
        for p in e
            .get("providers_queried")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            let base = stats::base_provider(p);
            *out.entry(base.to_string())
                .or_default()
                .entry(month.clone())
                .or_default() += stats::estimated_cost_per_call(base);
        }
    }
    for months in out.values_mut() {
        for v in months.values_mut() {
            *v = stats::round_usd(*v);
        }
    }
    out
}

/// Month of the oldest daily search log (`searches_YYYY-MM-DD.jsonl`).
fn log_coverage_start<I: IntoIterator<Item = String>>(names: I) -> Option<String> {
    names
        .into_iter()
        .filter_map(|n| {
            let date = n.strip_prefix("searches_")?.strip_suffix(".jsonl")?;
            month_of(date).map(str::to_string)
        })
        .min()
}

// ---- assembly ------------------------------------------------------------

fn build_row(
    provider: &str,
    configured: bool,
    history: Option<Result<History, String>>,
    estimates: Option<&BTreeMap<String, f64>>,
    coverage: Option<&str>,
    months: &[String],
) -> ProviderBilling {
    let estimate_cell = |m: &String| {
        if coverage.is_some_and(|c| m.as_str() >= c) {
            Cell {
                month: m.clone(),
                value: Some(estimates.and_then(|e| e.get(m)).copied().unwrap_or(0.0)),
                unit: Some("USD".to_string()),
                source: Source::Estimate,
            }
        } else {
            Cell {
                month: m.clone(),
                value: None,
                unit: None,
                source: Source::None,
            }
        }
    };
    let (unit, cells, error) = match history {
        Some(Ok(History::Monthly { unit, by_month })) => {
            let cells = months
                .iter()
                .map(|m| Cell {
                    month: m.clone(),
                    value: Some(by_month.get(m).copied().unwrap_or(0.0)),
                    unit: Some(unit.to_string()),
                    source: Source::Api,
                })
                .collect();
            (Some(unit.to_string()), cells, None)
        }
        Some(Ok(History::CurrentPeriod { unit, value })) => {
            let cells = months
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    if i == 0 {
                        Cell {
                            month: m.clone(),
                            value: Some(value),
                            unit: Some(unit.to_string()),
                            source: Source::ApiCurrentPeriod,
                        }
                    } else {
                        estimate_cell(m)
                    }
                })
                .collect();
            (Some(unit.to_string()), cells, None)
        }
        Some(Err(e)) => (None, months.iter().map(estimate_cell).collect(), Some(e)),
        None => (None, months.iter().map(estimate_cell).collect(), None),
    };
    ProviderBilling {
        provider: provider.to_string(),
        configured,
        unit,
        months: cells,
        error,
    }
}

/// Build the report: provider history fetched concurrently for configured
/// providers, local-log estimates for everything else. Every provider gets a
/// row; the keyless local scraper (stealth) is not a billed provider.
pub async fn collect(ctx: Arc<AppContext>) -> BillingReport {
    let today = dates::today_epoch_days();
    let months = last_months(today, MONTHS);

    let oldest_start = months
        .last()
        .map(|m| {
            let (y, mo) = year_month(m);
            dates::days_from_civil(y, mo, 1)
        })
        .unwrap_or(0);
    let window_days = (today as i64 - oldest_start + 1).max(1) as u64;
    let (entries, _) = stats::read_entries(window_days);
    let estimates = estimates_by_month(&entries);
    let coverage = std::fs::read_dir(logging::log_dir()).ok().and_then(|dir| {
        log_coverage_start(
            dir.flatten()
                .map(|e| e.file_name().to_string_lossy().to_string()),
        )
    });

    let all: Vec<(String, bool)> = providers::build_providers(&ctx)
        .iter()
        .filter(|p| p.name() != "stealth")
        .map(|p| (p.name().to_string(), p.is_configured()))
        .collect();

    let mut set = JoinSet::new();
    for (name, configured) in &all {
        if *configured && has_history(name) {
            let ctx = ctx.clone();
            let name = name.clone();
            let months = months.clone();
            set.spawn(async move {
                let key = resolve_provider_key(&ctx, &name);
                let r = timeout(USAGE_TIMEOUT, fetch_history(&ctx, &name, &key, &months))
                    .await
                    .unwrap_or_else(|_| Err("timed out".to_string()));
                (name, r)
            });
        }
    }
    let mut histories: BTreeMap<String, Result<History, String>> = BTreeMap::new();
    while let Some(Ok((name, r))) = set.join_next().await {
        histories.insert(name, r);
    }

    let mut rows: Vec<ProviderBilling> = all
        .iter()
        .map(|(name, configured)| {
            build_row(
                name,
                *configured,
                histories.remove(name),
                estimates.get(name),
                coverage.as_deref(),
                &months,
            )
        })
        .collect();
    rows.sort_by(|a, b| a.provider.cmp(&b.provider));

    BillingReport {
        months,
        estimates_since: coverage,
        providers: rows,
    }
}

// ---- rendering -----------------------------------------------------------

/// `6,962`, `48.2K`, `193M`, `1.2B`.
fn compact(v: f64) -> String {
    let one = |x: f64, suffix: &str| {
        if x >= 100.0 {
            format!("{x:.0}{suffix}")
        } else {
            format!("{x:.1}{suffix}")
        }
    };
    let a = v.abs();
    if a >= 1e9 {
        one(v / 1e9, "B")
    } else if a >= 1e6 {
        one(v / 1e6, "M")
    } else if a >= 1e4 {
        one(v / 1e3, "K")
    } else {
        let n = v.round() as i64;
        let digits = n.unsigned_abs().to_string();
        let mut out = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(c);
        }
        if n < 0 {
            format!("-{out}")
        } else {
            out
        }
    }
}

fn cell_text(c: &Cell) -> String {
    let Some(v) = c.value else {
        return "-".to_string();
    };
    let amount = if c.unit.as_deref() == Some("USD") {
        format!("${v:.2}")
    } else {
        compact(v)
    };
    match c.source {
        Source::Api => amount,
        Source::ApiCurrentPeriod => format!("{amount}*"),
        Source::Estimate => format!("~{amount}"),
        Source::None => "-".to_string(),
    }
}

pub fn render_human(r: &BillingReport) {
    use owo_colors::OwoColorize;
    const W: usize = 10;
    eprintln!(
        "\n{}  monthly usage, last {} months (UTC), newest first\n",
        "billing".bold().cyan(),
        r.months.len()
    );
    let mut header = format!("  {:<12} {:<9}", "provider", "unit");
    for m in &r.months {
        header.push_str(&format!(" {m:>W$}"));
    }
    println!("{}", header.bold());

    let any = |s: Source| {
        r.providers
            .iter()
            .any(|p| p.months.iter().any(|c| c.source == s))
    };
    let (has_current, has_estimate, has_none) = (
        any(Source::ApiCurrentPeriod),
        any(Source::Estimate),
        any(Source::None),
    );

    for p in &r.providers {
        let name = format!("{:<12}", p.provider);
        let name = if p.configured {
            name.bold().to_string()
        } else {
            name.dimmed().to_string()
        };
        let mut line = format!("  {name} {:<9}", p.unit.as_deref().unwrap_or("-"));
        for c in &p.months {
            let text = format!("{:>W$}", cell_text(c));
            let text = match c.source {
                Source::Api | Source::ApiCurrentPeriod => text.green().to_string(),
                Source::Estimate => text.to_string(),
                Source::None => text.dimmed().to_string(),
            };
            line.push(' ');
            line.push_str(&text);
        }
        println!("{line}");
    }

    println!();
    if has_current {
        println!("  * current billing cycle so far (may not align with the calendar month)");
    }
    if has_estimate {
        println!("  ~ estimate in USD from local search logs (`search stats` price table)");
    }
    if has_none {
        let since = r
            .estimates_since
            .as_deref()
            .map(|m| format!("; local logs start {m}"))
            .unwrap_or_else(|| "; no local search logs".to_string());
        println!("  - no data{since}");
    }
    for p in r.providers.iter().filter(|p| p.error.is_some()) {
        println!(
            "  {}: {}",
            p.provider,
            p.error.as_deref().unwrap_or_default().red()
        );
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn months_oct_2026() -> Vec<String> {
        last_months(dates::days_from_civil(2026, 10, 31) as u64, MONTHS)
    }

    #[test]
    fn last_months_newest_first() {
        assert_eq!(
            months_oct_2026(),
            ["2026-10", "2026-09", "2026-08", "2026-07", "2026-06", "2026-05"]
        );
    }

    #[test]
    fn last_months_crosses_year_boundary() {
        let today = dates::days_from_civil(2026, 2, 1) as u64;
        assert_eq!(
            last_months(today, 4),
            ["2026-02", "2026-01", "2025-12", "2025-11"]
        );
        assert_eq!(next_month("2025-12"), "2026-01");
        assert_eq!(next_month("2026-10"), "2026-11");
    }

    #[test]
    fn jina_pages_bucket_consumption_and_stop_past_window() {
        // Shape of live paginated_transaction records (newest first).
        let page = json!([
            {"type": "consume", "quantity": 28, "created_at": "2026-10-31T20:26:14+00:00",
             "metadata": {"model_name": "reader-crawl", "time_period": "2026-10-31T20:00:00+00:00"}},
            {"type": "consume", "quantity": 1000, "created_at": "2026-10-01T00:05:00+00:00",
             "metadata": {"time_period": "2026-09-30T23:00:00+00:00"}},
            {"type": "purchase", "quantity": 1000000000, "created_at": "2026-06-08T19:54:58+00:00",
             "metadata": {"product_name": "1B tokens"}},
            {"type": "consume", "quantity": 5, "created_at": "2026-06-02T00:00:00+00:00",
             "metadata": {"time_period": "2026-06-02T00:00:00+00:00"}},
            {"type": "consume", "quantity": 7, "created_at": "2026-04-30T00:00:00+00:00",
             "metadata": {"time_period": "2026-04-30T00:00:00+00:00"}},
        ]);
        let mut by_month = BTreeMap::new();
        let stop = jina_accumulate(&page, "2026-05", &mut by_month);
        assert!(stop);
        // time_period, not created_at, decides the month; purchases and
        // records before the window are not usage.
        assert_eq!(
            by_month,
            BTreeMap::from([
                ("2026-06".to_string(), 5.0),
                ("2026-09".to_string(), 1000.0),
                ("2026-10".to_string(), 28.0),
            ])
        );
    }

    #[test]
    fn jina_full_page_inside_window_keeps_paging() {
        let rec = json!({"type": "consume", "quantity": 1,
            "metadata": {"time_period": "2026-10-01T00:00:00+00:00"}});
        let page = Value::Array(vec![rec; JINA_PAGE]);
        let mut by_month = BTreeMap::new();
        assert!(!jina_accumulate(&page, "2026-05", &mut by_month));
        assert_eq!(by_month["2026-10"], JINA_PAGE as f64);
    }

    #[test]
    fn firecrawl_periods_by_start_month() {
        let v = json!({"success": true, "periods": [
            {"startDate": "2026-09-01T00:00:00Z", "endDate": "2026-09-30T23:59:59Z", "apiKey": null, "totalCredits": 1000},
            {"startDate": "2026-10-01T00:00:00Z", "endDate": "2026-10-31T23:59:59Z", "apiKey": null, "totalCredits": 250}
        ]});
        assert_eq!(
            firecrawl_history_from(&v),
            History::Monthly {
                unit: "credits",
                by_month: BTreeMap::from([
                    ("2026-09".to_string(), 1000.0),
                    ("2026-10".to_string(), 250.0)
                ])
            }
        );
    }

    #[test]
    fn xai_request_spans_window_and_series_are_summed() {
        let req = xai_usage_request("2026-05", "2026-11");
        let range = &req["analyticsRequest"]["timeRange"];
        assert_eq!(range["startTime"], "2026-05-01 00:00:00");
        assert_eq!(range["endTime"], "2026-11-01 00:00:00");
        assert_eq!(req["analyticsRequest"]["timeUnit"], "TIME_UNIT_MONTH");

        let v = json!({"timeSeries": [
            {"group": ["Chat grok-4.3"], "dataPoints": [
                {"timestamp": "2026-09-01T00:00:00Z", "values": [0.75]},
                {"timestamp": "2026-10-01T00:00:00Z", "values": [1.5]}]},
            {"group": ["X Search"], "dataPoints": [
                {"timestamp": "2026-10-01T00:00:00Z", "values": ["0.25"]}]}
        ], "limitReached": false});
        assert_eq!(
            xai_history_from(&v),
            History::Monthly {
                unit: "USD",
                by_month: BTreeMap::from([
                    ("2026-09".to_string(), 0.75),
                    ("2026-10".to_string(), 1.75)
                ])
            }
        );
    }

    #[test]
    fn current_period_parsers() {
        let tavily = json!({"key": {"usage": 6962}, "account": {"current_plan": "Researcher",
            "plan_usage": 6962, "plan_limit": 1000, "paygo_usage": 5948, "paygo_limit": null}});
        assert_eq!(
            tavily_history_from(&tavily),
            Ok(History::CurrentPeriod {
                unit: "credits",
                value: 6962.0
            })
        );
        let serpapi = json!({"plan_searches_left": 900, "this_month_usage": 100});
        assert_eq!(
            serpapi_history_from(&serpapi),
            Ok(History::CurrentPeriod {
                unit: "searches",
                value: 100.0
            })
        );
        assert!(serpapi_history_from(&json!({})).is_err());
    }

    #[test]
    fn estimates_bucket_fresh_calls_by_utc_month() {
        let oct = dates::days_from_civil(2026, 10, 31) as u64 * 86400 + 3600;
        let sep = dates::days_from_civil(2026, 9, 30) as u64 * 86400 + 86399;
        let entries = vec![
            json!({"ts": sep, "providers_queried": ["brave", "brave_llm_context"]}),
            json!({"ts": oct, "providers_queried": ["brave"]}),
            json!({"ts": oct, "cached": true, "providers_queried": ["brave"]}),
        ];
        let est = estimates_by_month(&entries);
        let brave = &est["brave"];
        let per_call = stats::estimated_cost_per_call("brave");
        assert!((brave["2026-09"] - 2.0 * per_call).abs() < 1e-12);
        assert!((brave["2026-10"] - per_call).abs() < 1e-12);
    }

    #[test]
    fn estimates_carry_no_float_noise() {
        let ts = dates::days_from_civil(2026, 10, 1) as u64 * 86400;
        let entries: Vec<Value> = (0..35)
            .map(|_| json!({"ts": ts, "providers_queried": ["brave"]}))
            .collect();
        let v = estimates_by_month(&entries)["brave"]["2026-10"];
        assert_eq!(v, stats::round_usd(v));
        assert_eq!(v.to_string().len(), stats::round_usd(v).to_string().len());
        assert!(v.to_string().len() <= 10, "{v}");
    }

    #[test]
    fn coverage_is_month_of_oldest_daily_log() {
        let names = [
            "balances.jsonl",
            "searches_2026-10-31.jsonl",
            "searches_2026-08-15.jsonl",
            "searches_garbage.jsonl",
        ]
        .map(String::from);
        assert_eq!(log_coverage_start(names), Some("2026-08".to_string()));
        assert_eq!(log_coverage_start(Vec::<String>::new()), None);
    }

    #[test]
    fn rows_never_mix_sources_silently() {
        let months = months_oct_2026();
        let est = BTreeMap::from([("2026-09".to_string(), 0.42)]);

        // Current-period provider: API in the current month, estimates after.
        let row = build_row(
            "tavily",
            true,
            Some(Ok(History::CurrentPeriod {
                unit: "credits",
                value: 6962.0,
            })),
            Some(&est),
            Some("2026-08"),
            &months,
        );
        let sources: Vec<Source> = row.months.iter().map(|c| c.source).collect();
        assert_eq!(
            sources,
            [
                Source::ApiCurrentPeriod,
                Source::Estimate,
                Source::Estimate,
                Source::None,
                Source::None,
                Source::None
            ]
        );
        let texts: Vec<String> = row.months.iter().map(cell_text).collect();
        assert_eq!(texts, ["6,962*", "~$0.42", "~$0.00", "-", "-", "-"]);

        // Monthly API history: every cell from the API, gaps are zero usage.
        let row = build_row(
            "jina",
            true,
            Some(Ok(History::Monthly {
                unit: "tokens",
                by_month: BTreeMap::from([("2026-10".to_string(), 193_268_530.0)]),
            })),
            None,
            None,
            &months,
        );
        assert_eq!(row.unit.as_deref(), Some("tokens"));
        assert!(row.months.iter().all(|c| c.source == Source::Api));
        assert_eq!(cell_text(&row.months[0]), "193M");
        assert_eq!(cell_text(&row.months[1]), "0");

        // A failed fetch keeps the error and falls back to estimates.
        let row = build_row(
            "xai",
            true,
            Some(Err("HTTP 401".to_string())),
            None,
            Some("2026-10"),
            &months,
        );
        assert_eq!(row.error.as_deref(), Some("HTTP 401"));
        assert_eq!(row.months[0].source, Source::Estimate);
        assert_eq!(row.months[1].source, Source::None);
    }

    #[test]
    fn compact_numbers() {
        assert_eq!(compact(0.0), "0");
        assert_eq!(compact(6962.0), "6,962");
        assert_eq!(compact(48_210.0), "48.2K");
        assert_eq!(compact(5_842_085.0), "5.8M");
        assert_eq!(compact(193_268_530.0), "193M");
        assert_eq!(compact(1_200_000_000.0), "1.2B");
    }
}
