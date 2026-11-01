//! `search stats` — local analytics over the JSONL search logs: volume,
//! modes, per-provider calls/failures, estimated spend, cache-hit rate, and
//! an extract-click-through table (which providers' results actually get
//! read), plus per-provider signal from the raw result rows and relevance
//! marks. Everything is computed from disk; nothing leaves the machine.

use crate::engine::normalize_url;
use crate::logging;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Rough per-call cost estimates in USD, from provider pricing pages
/// (2026-07). Estimates only — `search usage` balance snapshots are ground
/// truth where available. Keyed by base provider name.
/// Round a summed USD estimate to a micro-dollar, below any per-call
/// price, so float accumulation noise (0.17500000000000007) never shows.
pub(crate) fn round_usd(v: f64) -> f64 {
    (v * 1e6).round() / 1e6
}

pub(crate) fn estimated_cost_per_call(provider: &str) -> f64 {
    match provider {
        "serper" => 0.001,
        "jina" => 0.0005,
        "firecrawl" => 0.002,
        "brave" => 0.005,
        // Search API `fast` mode: $1 per 1,000 requests (docs, 2026-10).
        "parallel" => 0.001,
        "linkup" => 0.0055,
        "kagi" => 0.012,
        "exa" => 0.007,
        "tavily" => 0.010,
        "serpapi" => 0.010,
        // Agent API `fast` effort: $0.001 Fast Search call + ~5-10k input
        // and ~500 output tokens on gpt-6-luna at the 2x priority tier (2026-10).
        "perplexity" => 0.003,
        "browserless" => 0.004,
        // grok-4.3 tokens (~$0.02) plus X Search at $5 per 1k posts fetched
        // (~10-20 posts per call); xAI pricing as of 2026-09.
        "xai" => 0.08,
        _ => 0.0, // stealth and unknowns
    }
}

/// "brave_llm_context" bills as brave; "perplexity_fast" as perplexity.
pub(crate) fn base_provider(name: &str) -> &str {
    name.split('_').next().unwrap_or(name)
}

#[derive(Debug, Default, Serialize)]
pub struct ProviderStats {
    pub calls: u64,
    pub failures: u64,
    pub cancelled: u64,
    pub results_contributed: u64,
    pub estimated_spend_usd: f64,
    /// Times an extract/scrape later ran on a URL this provider returned —
    /// the closest local proxy for "an agent found this result useful".
    pub result_read_through: u64,
}

/// What a provider's results were worth over the window. Except `results`,
/// counts are distinct canonical URLs.
#[derive(Debug, Default, Serialize)]
pub struct ProviderSignal {
    /// Result rows returned on fresh runs, repeats included.
    pub results: u64,
    pub urls: u64,
    /// URLs no other provider returned.
    pub unique: u64,
    /// URLs this provider returned that were later marked relevant.
    pub relevant: u64,
    /// Marked URLs no other provider returned: what would be lost without it.
    pub unique_relevant: u64,
    /// URLs this provider returned that were later extracted or scraped.
    pub extracted: u64,
    /// relevant / urls.
    pub precision: f64,
    /// Spend on search calls only: extract and scrape spend buys page reads,
    /// not results.
    pub search_spend_usd: f64,
    pub cost_per_relevant_usd: Option<f64>,
    pub cost_per_unique_relevant_usd: Option<f64>,
}

#[derive(Debug, Default, Serialize)]
pub struct Signal {
    pub providers: BTreeMap<String, ProviderSignal>,
    pub marked_urls: u64,
    /// Marked URLs no provider returned in the lookback, such as pages found
    /// through another search tool or a link in a document.
    pub marked_unattributed: u64,
}

#[derive(Debug, Deserialize)]
struct ResultRow {
    ts: u64,
    provider: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct MarkRow {
    ts: u64,
    url: String,
}

/// A mark or extract credits a provider only for results from this long
/// before it, so an unrelated search long ago earns nothing.
const CREDIT_LOOKBACK_SECS: u64 = 7 * 86400;

#[derive(Debug, Serialize)]
pub struct StatsReport {
    pub window_days: u64,
    pub searches: u64,
    pub cache_hits: u64,
    pub cache_hit_rate: f64,
    pub by_mode: BTreeMap<String, u64>,
    pub by_provider: BTreeMap<String, ProviderStats>,
    pub estimated_total_spend_usd: f64,
    pub top_queries: Vec<(String, u64)>,
    /// Latest known balance per provider from `search usage` snapshots,
    /// with the change across the window (negative = burn).
    pub balances: BTreeMap<String, BalanceTrend>,
    pub provider_signal: Signal,
    pub log_files_read: u64,
}

#[derive(Debug, Serialize)]
pub struct BalanceTrend {
    pub latest: f64,
    pub unit: String,
    pub change_in_window: Option<f64>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Every log line in the window, oldest first.
pub(crate) fn read_entries(days: u64) -> (Vec<serde_json::Value>, u64) {
    let (mut entries, files) = read_rows("searches_", days, |v: &serde_json::Value| {
        v.get("ts").and_then(|t| t.as_u64()).unwrap_or(0)
    });
    entries.sort_by_key(|v| v.get("ts").and_then(|t| t.as_u64()).unwrap_or(0));
    (entries, files)
}

/// True when a daily log named `<prefix>YYYY-MM-DD.jsonl` ended before `cutoff`.
fn file_before_cutoff(name: &str, prefix: &str, cutoff: u64) -> bool {
    let Some(date) = name
        .strip_prefix(prefix)
        .and_then(|n| n.strip_suffix(".jsonl"))
    else {
        return false;
    };
    let mut parts = date.split('-').map(str::parse::<i64>);
    let (Some(Ok(y)), Some(Ok(m)), Some(Ok(d))) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let next_day = crate::dates::days_from_civil(y, m as u32, d as u32) + 1;
    next_day.saturating_mul(86400) < cutoff as i64
}

/// Rows of every `<prefix>*.jsonl` log inside the window, skipping files that
/// ended before it so a long history costs nothing.
fn read_rows<T: DeserializeOwned>(
    prefix: &str,
    days: u64,
    ts: impl Fn(&T) -> u64,
) -> (Vec<T>, u64) {
    let cutoff = now_secs().saturating_sub(days.saturating_mul(86400));
    let mut rows = Vec::new();
    let mut files = 0u64;
    let Ok(dir) = std::fs::read_dir(logging::log_dir()) else {
        return (rows, files);
    };
    for entry in dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with(prefix)
            || !name.ends_with(".jsonl")
            || file_before_cutoff(&name, prefix, cutoff)
        {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        files += 1;
        rows.extend(
            content
                .lines()
                .filter_map(|line| serde_json::from_str::<T>(line).ok())
                .filter(|row| ts(row) >= cutoff),
        );
    }
    (rows, files)
}

pub fn compute(days: u64) -> StatsReport {
    let (entries, log_files_read) = read_entries(days);
    let (results, _) = read_rows("results_", days, |r: &ResultRow| r.ts);
    let (marks, _) = read_rows("marks_", days, |m: &MarkRow| m.ts);
    let signal = provider_signal(&results, &marks, &entries);
    aggregate(
        &entries,
        days,
        log_files_read,
        read_balance_trends(days),
        signal,
    )
}

/// `mode` as logged: failed extracts carry a `+reddit-reader=...` suffix.
fn is_extract_mode(mode: &str) -> bool {
    matches!(mode.split('+').next(), Some("extract" | "scrape"))
}

/// Providers credited for `events` (canonical URL, ts) pairs: `found` holds
/// url -> provider -> result timestamps. Yields each (url, provider) once.
fn credited<'a>(
    events: &'a HashMap<String, Vec<u64>>,
    found: &'a HashMap<String, HashMap<String, Vec<u64>>>,
) -> impl Iterator<Item = (&'a str, &'a str)> {
    events.iter().flat_map(move |(url, event_times)| {
        found.get(url).into_iter().flat_map(move |providers| {
            providers
                .iter()
                .filter(|(_, result_times)| {
                    event_times.iter().any(|event| {
                        result_times
                            .iter()
                            .any(|r| r <= event && event - r <= CREDIT_LOOKBACK_SECS)
                    })
                })
                .map(move |(provider, _)| (url.as_str(), provider.as_str()))
        })
    })
}

fn provider_signal(
    results: &[ResultRow],
    marks: &[MarkRow],
    entries: &[serde_json::Value],
) -> Signal {
    let mut signal = Signal::default();
    let mut found: HashMap<String, HashMap<String, Vec<u64>>> = HashMap::new();
    for row in results {
        let base = base_provider(&row.provider).to_string();
        signal.providers.entry(base.clone()).or_default().results += 1;
        found
            .entry(normalize_url(&row.url))
            .or_default()
            .entry(base)
            .or_default()
            .push(row.ts);
    }
    for providers in found.values() {
        let sole = providers.len() == 1;
        for p in providers.keys() {
            let stat = signal.providers.entry(p.clone()).or_default();
            stat.urls += 1;
            stat.unique += u64::from(sole);
        }
    }

    let mut marked: HashMap<String, Vec<u64>> = HashMap::new();
    for mark in marks {
        marked
            .entry(normalize_url(&mark.url))
            .or_default()
            .push(mark.ts);
    }
    let mut attributed = HashSet::new();
    for (url, provider) in credited(&marked, &found) {
        attributed.insert(url);
        let stat = signal.providers.entry(provider.to_string()).or_default();
        stat.relevant += 1;
        stat.unique_relevant += u64::from(found[url].len() == 1);
    }
    signal.marked_urls = marked.len() as u64;
    signal.marked_unattributed = (marked.len() - attributed.len()) as u64;

    let mut extracted: HashMap<String, Vec<u64>> = HashMap::new();
    for e in entries {
        let mode = e.get("mode").and_then(|m| m.as_str()).unwrap_or("");
        let ts = e.get("ts").and_then(|t| t.as_u64()).unwrap_or(0);
        let succeeded = e.get("result_count").and_then(|n| n.as_u64()).unwrap_or(0) > 0;
        if is_extract_mode(mode) {
            if succeeded {
                let url = e.get("query").and_then(|q| q.as_str()).unwrap_or("");
                extracted.entry(normalize_url(url)).or_default().push(ts);
            }
            continue;
        }
        // Entries from before result rows existed have no search_id; their
        // spend has no matching results and would inflate cost per result.
        let cached = e.get("cached").and_then(|c| c.as_bool()).unwrap_or(false);
        if cached || e.get("search_id").is_none() {
            continue;
        }
        for p in str_array(e, "providers_queried") {
            let base = base_provider(&p).to_string();
            let cost = estimated_cost_per_call(&base);
            signal.providers.entry(base).or_default().search_spend_usd += cost;
        }
    }
    for (_, provider) in credited(&extracted, &found) {
        signal
            .providers
            .entry(provider.to_string())
            .or_default()
            .extracted += 1;
    }

    let per = |spend: f64, n: u64| (n > 0).then(|| round_usd(spend / n as f64));
    for p in signal.providers.values_mut() {
        p.search_spend_usd = round_usd(p.search_spend_usd);
        if p.urls > 0 {
            p.precision = p.relevant as f64 / p.urls as f64;
        }
        p.cost_per_relevant_usd = per(p.search_spend_usd, p.relevant);
        p.cost_per_unique_relevant_usd = per(p.search_spend_usd, p.unique_relevant);
    }
    signal
}

fn aggregate(
    entries: &[serde_json::Value],
    days: u64,
    log_files_read: u64,
    balances: BTreeMap<String, BalanceTrend>,
    provider_signal: Signal,
) -> StatsReport {
    let mut searches = 0u64;
    let mut cache_hits = 0u64;
    let mut by_mode: BTreeMap<String, u64> = BTreeMap::new();
    let mut by_provider: BTreeMap<String, ProviderStats> = BTreeMap::new();
    let mut query_counts: HashMap<String, u64> = HashMap::new();
    // url -> providers that returned it, for read-through attribution.
    let mut url_sources: HashMap<String, HashSet<String>> = HashMap::new();

    for e in entries {
        searches += 1;
        let cached = e.get("cached").and_then(|c| c.as_bool()).unwrap_or(false);
        if cached {
            cache_hits += 1;
        }
        let mode = e.get("mode").and_then(|m| m.as_str()).unwrap_or("?");
        *by_mode.entry(mode.to_string()).or_default() += 1;

        let query = e.get("query").and_then(|q| q.as_str()).unwrap_or("");
        *query_counts.entry(query.to_lowercase()).or_default() += 1;

        // Read-through: an extract/scrape whose target URL was previously
        // returned by a search credits every provider that returned it.
        if matches!(mode, "extract" | "scrape")
            && let Some(sources) = url_sources.get(&normalize_url(query))
        {
            for s in sources.clone() {
                by_provider.entry(s).or_default().result_read_through += 1;
            }
        }

        // Billing/contribution only counts fresh runs.
        if !cached {
            for p in str_array(e, "providers_queried") {
                let base = base_provider(&p).to_string();
                let stat = by_provider.entry(base.clone()).or_default();
                stat.calls += 1;
                stat.estimated_spend_usd += estimated_cost_per_call(&base);
            }
            for p in str_array(e, "providers_failed") {
                by_provider
                    .entry(base_provider(&p).to_string())
                    .or_default()
                    .failures += 1;
            }
            for p in str_array(e, "providers_cancelled") {
                by_provider
                    .entry(base_provider(&p).to_string())
                    .or_default()
                    .cancelled += 1;
            }
            if let Some(obj) = e.get("provider_results").and_then(|o| o.as_object()) {
                for (p, n) in obj {
                    by_provider
                        .entry(base_provider(p).to_string())
                        .or_default()
                        .results_contributed += n.as_u64().unwrap_or(0);
                }
            }
        }

        // `urls` is the first ten results and `sources` every result, in
        // the same order, so index i pairs a URL with the provider that
        // returned it.
        let sources = str_array(e, "sources");
        for (url, source) in str_array(e, "urls").iter().zip(&sources) {
            url_sources
                .entry(normalize_url(url))
                .or_default()
                .insert(base_provider(source).to_string());
        }
    }

    for p in by_provider.values_mut() {
        p.estimated_spend_usd = round_usd(p.estimated_spend_usd);
    }
    let estimated_total_spend_usd =
        round_usd(by_provider.values().map(|p| p.estimated_spend_usd).sum());

    let mut top_queries: Vec<(String, u64)> = query_counts.into_iter().collect();
    top_queries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    top_queries.truncate(10);
    top_queries.retain(|(_, n)| *n > 1);

    StatsReport {
        window_days: days,
        searches,
        cache_hits,
        cache_hit_rate: if searches > 0 {
            cache_hits as f64 / searches as f64
        } else {
            0.0
        },
        by_mode,
        by_provider,
        estimated_total_spend_usd,
        top_queries,
        balances,
        provider_signal,
        log_files_read,
    }
}

fn str_array(e: &serde_json::Value, key: &str) -> Vec<String> {
    e.get(key)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Latest balance per provider plus the delta across the window — actual
/// burn, measured, not estimated. Fed by `search usage` snapshots.
fn read_balance_trends(days: u64) -> BTreeMap<String, BalanceTrend> {
    let cutoff = now_secs().saturating_sub(days.saturating_mul(86400));
    let path = logging::log_dir().join("balances.jsonl");
    let Ok(content) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    // provider -> (first_in_window, latest, unit)
    let mut trends: BTreeMap<String, (Option<f64>, f64, String)> = BTreeMap::new();
    for line in content.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let ts = v.get("ts").and_then(|t| t.as_u64()).unwrap_or(0);
        let Some(balances) = v.get("balances").and_then(|b| b.as_object()) else {
            continue;
        };
        for (provider, info) in balances {
            let Some(remaining) = info.get("credits_remaining").and_then(|c| c.as_f64()) else {
                continue;
            };
            let unit = info
                .get("unit")
                .and_then(|u| u.as_str())
                .unwrap_or("credits")
                .to_string();
            let entry = trends
                .entry(provider.clone())
                .or_insert((None, remaining, unit.clone()));
            if ts >= cutoff && entry.0.is_none() {
                entry.0 = Some(remaining);
            }
            entry.1 = remaining;
            entry.2 = unit;
        }
    }
    trends
        .into_iter()
        .map(|(p, (first, latest, unit))| {
            (
                p,
                BalanceTrend {
                    latest,
                    unit,
                    change_in_window: first.map(|f| latest - f),
                },
            )
        })
        .collect()
}

const LOG_PREFIXES: [&str; 3] = ["searches_", "results_", "marks_"];

/// Delete log files older than `days`. Returns files removed.
pub fn prune_logs(days: u64) -> usize {
    let cutoff = now_secs().saturating_sub(days.saturating_mul(86400));
    let Ok(dir) = std::fs::read_dir(logging::log_dir()) else {
        return 0;
    };
    let mut removed = 0;
    for entry in dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !LOG_PREFIXES.iter().any(|p| name.starts_with(p)) || !name.ends_with(".jsonl") {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
            .is_some_and(|d| d.as_secs() < cutoff);
        if old && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

fn render_signal(s: &Signal) {
    use owo_colors::OwoColorize;
    if s.providers.is_empty() {
        return;
    }
    println!(
        "\nprovider signal ({} URLs marked relevant, {} from no logged provider):",
        s.marked_urls, s.marked_unattributed
    );
    println!(
        "  {:<12} {:>8} {:>8} {:>6} {:>8} {:>8} {:>7} {:>6} {:>6} {:>7}",
        "provider",
        "uniq-rel",
        "relevant",
        "prec",
        "$/rel",
        "$/uniq",
        "results",
        "urls",
        "unique",
        "extract"
    );
    let mut rows: Vec<_> = s.providers.iter().collect();
    rows.sort_by(|a, b| {
        (b.1.unique_relevant, b.1.relevant, a.0).cmp(&(a.1.unique_relevant, a.1.relevant, b.0))
    });
    let usd = |v: Option<f64>| v.map_or_else(|| "-".to_string(), |c| format!("{c:.3}"));
    for (name, p) in rows {
        println!(
            "  {:<12} {:>8} {:>8} {:>5.1}% {:>8} {:>8} {:>7} {:>6} {:>6} {:>7}",
            name,
            p.unique_relevant.bold(),
            p.relevant,
            p.precision * 100.0,
            usd(p.cost_per_relevant_usd),
            usd(p.cost_per_unique_relevant_usd),
            p.results,
            p.urls,
            p.unique,
            p.extracted
        );
    }
}

pub fn render_human(r: &StatsReport) {
    use owo_colors::OwoColorize;
    eprintln!(
        "\n{}  last {} days — {} searches, {:.0}% cache hits\n",
        "stats".bold().cyan(),
        r.window_days,
        r.searches,
        r.cache_hit_rate * 100.0
    );
    println!("modes:");
    for (mode, n) in &r.by_mode {
        println!("  {mode:<10} {n}");
    }
    println!("\nproviders (fresh calls only):");
    println!(
        "  {:<12} {:>6} {:>6} {:>7} {:>8} {:>10} {:>6}",
        "provider", "calls", "fail", "cancel", "results", "~spend", "read"
    );
    for (p, s) in &r.by_provider {
        println!(
            "  {:<12} {:>6} {:>6} {:>7} {:>8} {:>9.2}$ {:>6}",
            p,
            s.calls,
            s.failures,
            s.cancelled,
            s.results_contributed,
            s.estimated_spend_usd,
            s.result_read_through
        );
    }
    println!(
        "\nestimated total spend: ${:.2} (estimates; run `search usage` for real balances)",
        r.estimated_total_spend_usd
    );
    if !r.balances.is_empty() {
        println!("\nbalances (from usage snapshots):");
        for (p, b) in &r.balances {
            let delta = b
                .change_in_window
                .map(|d| format!(" ({d:+.1} this window)"))
                .unwrap_or_default();
            println!("  {:<12} {} {}{}", p, b.latest, b.unit, delta);
        }
    }
    render_signal(&r.provider_signal);
    if !r.top_queries.is_empty() {
        println!("\nrepeated queries:");
        for (q, n) in &r.top_queries {
            println!("  {n}x  {q}");
        }
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn read_through_credits_only_the_provider_that_returned_the_url() {
        let entries = vec![
            json!({"ts": 1, "query": "q", "mode": "general",
                   "providers_queried": ["brave", "exa"],
                   "sources": ["brave", "exa"],
                   "urls": ["https://b.test/", "https://e.test/"]}),
            json!({"ts": 2, "query": "https://b.test", "mode": "extract",
                   "providers_queried": ["jina"]}),
        ];
        let r = aggregate(&entries, 7, 1, BTreeMap::new(), Signal::default());
        assert_eq!(r.by_provider["brave"].result_read_through, 1);
        assert_eq!(r.by_provider["exa"].result_read_through, 0);
    }

    #[test]
    fn failed_search_counts_as_a_search_with_failures() {
        let entries = vec![json!({"ts": 1, "query": "q", "mode": "general",
            "result_count": 0, "providers_queried": ["serpapi"],
            "providers_failed": ["serpapi"]})];
        let r = aggregate(&entries, 7, 1, BTreeMap::new(), Signal::default());
        assert_eq!(r.searches, 1);
        assert_eq!(r.by_provider["serpapi"].failures, 1);
        assert_eq!(r.by_provider["serpapi"].calls, 1);
    }

    const DAY: u64 = 86400;

    fn row(ts: u64, provider: &str, url: &str) -> ResultRow {
        ResultRow {
            ts,
            provider: provider.into(),
            url: url.into(),
        }
    }

    fn mark(ts: u64, url: &str) -> MarkRow {
        MarkRow {
            ts,
            url: url.into(),
        }
    }

    #[test]
    fn unique_relevant_needs_a_sole_provider() {
        let results = [
            row(10, "brave", "https://shared.test/a"),
            row(10, "exa", "https://www.shared.test/a?utm_source=x#s"),
            row(10, "exa", "https://only-exa.test/b"),
        ];
        let marks = [
            mark(20, "http://shared.test/a/"),
            mark(20, "https://only-exa.test/b"),
        ];
        let s = provider_signal(&results, &marks, &[]);
        let (brave, exa) = (&s.providers["brave"], &s.providers["exa"]);
        assert_eq!(
            (brave.relevant, brave.unique_relevant, brave.unique),
            (1, 0, 0)
        );
        assert_eq!((exa.relevant, exa.unique_relevant, exa.unique), (2, 1, 1));
        assert_eq!((exa.results, exa.urls), (2, 2));
        assert_eq!(exa.precision, 1.0);
        assert_eq!((s.marked_urls, s.marked_unattributed), (2, 0));
    }

    #[test]
    fn repeats_count_as_results_but_one_url() {
        let results = [
            row(10, "exa", "https://a.test"),
            row(11, "exa", "https://a.test/"),
        ];
        let marks = [mark(20, "https://a.test"), mark(21, "https://a.test")];
        let s = provider_signal(&results, &marks, &[]);
        let exa = &s.providers["exa"];
        assert_eq!((exa.results, exa.urls, exa.relevant), (2, 1, 1));
    }

    #[test]
    fn marks_only_credit_results_from_the_lookback_before_them() {
        let results = [
            row(100 * DAY, "late", "https://a.test"),
            row(50 * DAY, "stale", "https://a.test"),
            row(95 * DAY, "recent", "https://a.test"),
        ];
        let s = provider_signal(&results, &[mark(99 * DAY, "https://a.test")], &[]);
        assert_eq!(s.providers["late"].relevant, 0);
        assert_eq!(s.providers["stale"].relevant, 0);
        assert_eq!(s.providers["recent"].relevant, 1);
    }

    #[test]
    fn a_mark_no_provider_returned_is_unattributed() {
        let s = provider_signal(&[], &[mark(1, "https://a.test")], &[]);
        assert_eq!((s.marked_urls, s.marked_unattributed), (1, 1));
        assert!(s.providers.is_empty());
    }

    #[test]
    fn only_successful_extracts_count() {
        let results = [
            row(10, "exa", "https://a.test"),
            row(10, "exa", "https://b.test"),
        ];
        let entries = [
            json!({"ts": 20, "query": "https://a.test", "mode": "extract", "result_count": 1,
                   "providers_queried": ["jina"]}),
            json!({"ts": 20, "query": "https://b.test", "mode": "extract+reddit-reader=jina",
                   "result_count": 0, "providers_queried": ["jina"]}),
        ];
        let s = provider_signal(&results, &[], &entries);
        assert_eq!(s.providers["exa"].extracted, 1);
    }

    #[test]
    fn cost_counts_fresh_logged_search_calls_only() {
        let results = [row(10, "exa", "https://a.test")];
        let entries = [
            json!({"ts": 10, "search_id": "s1", "mode": "general",
                   "providers_queried": ["exa", "jina"]}),
            json!({"ts": 11, "search_id": "s2", "mode": "general", "cached": true,
                   "providers_queried": ["exa"]}),
            // Pre-upgrade entry: no search_id, no matching rows.
            json!({"ts": 12, "mode": "general", "providers_queried": ["exa"]}),
            json!({"ts": 13, "search_id": "s3", "mode": "extract", "result_count": 1,
                   "query": "https://z.test", "providers_queried": ["jina"]}),
        ];
        let s = provider_signal(&results, &[mark(20, "https://a.test")], &entries);
        assert_eq!(s.providers["exa"].search_spend_usd, 0.007);
        assert_eq!(s.providers["exa"].cost_per_relevant_usd, Some(0.007));
        assert_eq!(s.providers["exa"].cost_per_unique_relevant_usd, Some(0.007));
        // A provider that was paid and returned nothing still shows up.
        assert_eq!(s.providers["jina"].search_spend_usd, 0.0005);
        assert_eq!(s.providers["jina"].cost_per_relevant_usd, None);
    }

    #[test]
    fn deep_mode_variants_roll_up_to_the_base_provider() {
        let results = [
            row(10, "brave", "https://a.test"),
            row(10, "brave_llm_context", "https://a.test"),
        ];
        let s = provider_signal(&results, &[mark(20, "https://a.test")], &[]);
        assert_eq!(s.providers.len(), 1);
        assert_eq!(s.providers["brave"].results, 2);
        assert_eq!(s.providers["brave"].unique_relevant, 1);
    }

    #[test]
    fn old_daily_files_are_skipped_by_name() {
        let cutoff = 20000 * DAY; // 2024-10-04 00:00 UTC.
        assert!(file_before_cutoff(
            "results_2024-10-02.jsonl",
            "results_",
            cutoff
        ));
        assert!(!file_before_cutoff(
            "results_2024-10-03.jsonl",
            "results_",
            cutoff
        ));
        assert!(!file_before_cutoff(
            "results_2024-10-04.jsonl",
            "results_",
            cutoff
        ));
        assert!(!file_before_cutoff(
            "results_junk.jsonl",
            "results_",
            cutoff
        ));
    }
}
