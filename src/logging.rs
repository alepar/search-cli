use crate::types::{ProviderFailure, SearchResponse};
use directories::ProjectDirs;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn log_dir() -> PathBuf {
    if let Some(proj) = ProjectDirs::from("", "", "search") {
        proj.data_dir().join("logs")
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("search")
            .join("logs")
    }
}

/// Local logging is on by default; SEARCH_LOG=off disables it entirely
/// (privacy opt-out — queries and result URLs otherwise land on disk).
fn logging_disabled() -> bool {
    std::env::var("SEARCH_LOG").is_ok_and(|v| v.eq_ignore_ascii_case("off"))
}

/// One log file per day per kind: searches_2026-03-22.jsonl
fn log_path(kind: &str) -> PathBuf {
    let date = crate::dates::epoch_days_to_date(crate::dates::today_epoch_days());
    log_dir().join(format!("{kind}_{date}.jsonl"))
}

/// Ties a searches_ line to its results_ rows: unique across concurrent agent
/// processes without a random-number dependency.
fn new_search_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{nanos:x}-{:x}", std::process::id())
}

/// Log a completed search (or cache replay) to the daily JSONL log file.
/// The entry carries a schema version (`v`) so `search stats` and external
/// analysis survive future field changes. The line is written with ONE
/// write_all call: concurrent agent invocations append to the same file, and
/// multi-write appends can interleave.
pub fn log_search(response: &SearchResponse) {
    let search_id = new_search_id();
    let ts = now_secs();
    let entry = serde_json::json!({
        "v": 1,
        "ts": ts,
        "search_id": search_id,
        "query": response.query,
        "mode": response.mode,
        "result_count": response.metadata.result_count,
        "elapsed_ms": response.metadata.elapsed_ms,
        "cached": response.metadata.cached,
        "providers_queried": response.metadata.providers_queried,
        "providers_failed": response.metadata.providers_failed,
        "providers_cancelled": response.metadata.providers_cancelled,
        "provider_results": response.metadata.provider_results,
        "failure_categories": failure_categories(&response.metadata.provider_failures),
        "answers": response.answers.iter().map(|a| &a.provider).collect::<Vec<_>>(),
        "warnings": response.metadata.warnings.len(),
        "sources": response.results.iter().map(|r| &r.source).collect::<Vec<_>>(),
        "urls": response.results.iter().take(10).map(|r| &r.url).collect::<Vec<_>>(),
    });
    append_search_entry(&entry);
    let rows = result_rows(&search_id, ts, response);
    if !rows.is_empty() {
        append_lines("results", &rows);
    }
}

/// One JSON line per result per provider: URL only, no snippet or content.
/// A fused URL has a row for each provider that found it, so `search stats`
/// can tell a unique find from a repeat. Pseudo-URLs are not web results.
fn result_rows(search_id: &str, ts: u64, response: &SearchResponse) -> String {
    let mut rows = String::new();
    for (provider, urls) in &response.provider_urls {
        let web_urls = urls.iter().filter(|u| u.starts_with("http"));
        for (rank, url) in web_urls.enumerate() {
            let row = serde_json::json!({
                "v": 1,
                "ts": ts,
                "search_id": search_id,
                "mode": response.mode,
                "provider": provider,
                "rank": rank + 1,
                "url": url,
            });
            rows.push_str(&row.to_string());
            rows.push('\n');
        }
    }
    rows
}

/// Record `urls` as found relevant. Returns the rows written: 0 when logging
/// is off or the write failed, so the command can report what happened.
pub fn log_marks(urls: &[String]) -> usize {
    let ts = now_secs();
    let mut rows = String::new();
    for url in urls {
        rows.push_str(&serde_json::json!({ "v": 1, "ts": ts, "url": url }).to_string());
        rows.push('\n');
    }
    if append_lines("marks", &rows) {
        urls.len()
    } else {
        0
    }
}

/// Log a search where every provider failed. It returns an error rather
/// than a response, but its calls were made (and billed), so `search stats`
/// must see them. Same schema as [`log_search`], with no results.
pub fn log_failed_search(query: &str, mode: &str, elapsed_ms: u128, failed: &[ProviderFailure]) {
    let providers: Vec<&str> = failed.iter().map(|f| f.provider.as_str()).collect();
    let entry = serde_json::json!({
        "v": 1,
        "ts": now_secs(),
        "search_id": new_search_id(),
        "query": query,
        "mode": mode,
        "result_count": 0,
        "elapsed_ms": elapsed_ms,
        "cached": false,
        "providers_queried": providers,
        "providers_failed": providers,
        "failure_categories": failure_categories(failed),
    });
    append_search_entry(&entry);
}

fn failure_categories(failed: &[ProviderFailure]) -> Vec<String> {
    failed
        .iter()
        .map(|f| format!("{}:{}", f.provider, f.category.as_str()))
        .collect()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn append_search_entry(entry: &serde_json::Value) {
    let mut line = serde_json::to_string(entry).unwrap_or_default();
    line.push('\n');
    append_lines("searches", &line);
}

/// Append to today's `kind` log. False when logging is off or the write
/// failed; a logging failure never reaches a search.
fn append_lines(kind: &str, lines: &str) -> bool {
    if logging_disabled() {
        return false;
    }
    let path = log_path(kind);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    append_to(&path, lines)
}

/// One write_all per batch, so concurrent appenders cannot interleave rows.
fn append_to(path: &Path, lines: &str) -> bool {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(lines.as_bytes()))
        .is_ok()
}

/// Append a `search usage` balance snapshot to balances.jsonl. Deltas
/// between snapshots give `search stats` measured credit burn — ground truth
/// the per-call estimates can't provide.
pub fn log_balances(balances: &serde_json::Value) {
    if logging_disabled() {
        return;
    }
    let dir = log_dir();
    let _ = fs::create_dir_all(&dir);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let entry = serde_json::json!({ "v": 1, "ts": now, "balances": balances });
    let mut line = serde_json::to_string(&entry).unwrap_or_default();
    line.push('\n');
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("balances.jsonl"))
    {
        let _ = file.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SearchResponse;

    fn response(provider_urls: Vec<(String, Vec<String>)>) -> SearchResponse {
        let mut r: SearchResponse = serde_json::from_value(serde_json::json!({
            "version": "1", "status": "success", "query": "q", "mode": "general",
            "results": [],
            "metadata": {"elapsed_ms": 1, "result_count": 0,
                         "providers_queried": [], "providers_failed": []},
        }))
        .unwrap();
        r.provider_urls = provider_urls;
        r
    }

    fn rows(text: &str) -> Vec<serde_json::Value> {
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn one_row_per_provider_result_with_url_only() {
        let r = response(vec![
            (
                "brave".into(),
                vec!["https://a.test".into(), "https://b.test".into()],
            ),
            ("exa".into(), vec!["https://a.test".into()]),
        ]);
        let got = rows(&result_rows("id1", 7, &r));
        assert_eq!(got.len(), 3);
        assert_eq!(got[1]["provider"], "brave");
        assert_eq!(got[1]["rank"], 2);
        assert_eq!(got[1]["url"], "https://b.test");
        assert_eq!(got[2]["provider"], "exa");
        assert_eq!(got[2]["rank"], 1);
        assert_eq!(got[2]["search_id"], "id1");
        assert_eq!(got[2]["mode"], "general");
        assert_eq!(got[2]["ts"], 7);
        assert!(got[0].get("snippet").is_none() && got[0].get("title").is_none());
    }

    #[test]
    fn pseudo_urls_are_not_logged_and_do_not_take_a_rank() {
        let r = response(vec![(
            "x".into(),
            vec!["about:blank".into(), "https://a.test".into()],
        )]);
        let got = rows(&result_rows("id", 1, &r));
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["rank"], 1);
    }

    #[test]
    fn a_cache_round_trip_drops_provider_urls_so_replays_log_no_rows() {
        let r = response(vec![("brave".into(), vec!["https://a.test".into()])]);
        assert!(!result_rows("id", 1, &r).is_empty());
        let cached: SearchResponse =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert!(result_rows("id", 1, &cached).is_empty());
    }

    #[test]
    fn concurrent_batches_never_interleave() {
        let dir = std::env::temp_dir().join(format!("search-log-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("results.jsonl");
        std::thread::scope(|s| {
            for t in 0..8 {
                let path = &path;
                s.spawn(move || {
                    let urls = (0..200)
                        .map(|i| format!("https://a.test/{t}/{i}"))
                        .collect();
                    let batch = result_rows("id", 1, &response(vec![(format!("p{t}"), urls)]));
                    for _ in 0..50 {
                        assert!(append_to(path, &batch));
                    }
                });
            }
        });
        let text = fs::read_to_string(&path).unwrap();
        let _ = fs::remove_dir_all(&dir);
        // Rows parse, and each batch is contiguous: a provider only changes
        // on a batch boundary.
        let got = rows(&text);
        assert_eq!(got.len(), 8 * 50 * 200);
        for batch in got.chunks(200) {
            assert!(batch.iter().all(|r| r["provider"] == batch[0]["provider"]));
        }
    }
}
