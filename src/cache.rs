use crate::types::SearchResponse;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CACHE_TTL_SECS: u64 = 300; // 5 minutes

fn cache_dir() -> PathBuf {
    if let Some(proj) = ProjectDirs::from("", "", "search") {
        proj.cache_dir().to_path_buf()
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(".cache").join("search")
    }
}

fn last_path() -> PathBuf {
    cache_dir().join("last.json")
}

/// Stable FNV-1a hash of (query, mode). Unlike `DefaultHasher`, the digest is
/// fixed across Rust versions, so cache files survive a toolchain upgrade
/// instead of being silently orphaned (and slowly leaking disk). The query is
/// hashed as given: URL modes are case-sensitive, and folding case would
/// replay one page for another.
fn stable_hash(query: &str, mode: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let feed = |h: &mut u64, bytes: &[u8]| {
        for &b in bytes {
            *h ^= b as u64;
            *h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    feed(&mut h, query.as_bytes());
    feed(&mut h, &[0]); // delimiter so ("ab","c") != ("a","bc")
    feed(&mut h, mode.as_bytes());
    h
}

/// Query-cache file prefixes, current first. `q_` used the unstable
/// DefaultHasher; `q2_` folded case and stored no query.
const QUERY_PREFIX: &str = "q3_";
const OLD_QUERY_PREFIXES: &[&str] = &["q_", "q2_"];

fn query_cache_path(dir: &Path, query: &str, mode: &str) -> PathBuf {
    dir.join(format!(
        "{QUERY_PREFIX}{:016x}.json",
        stable_hash(query, mode)
    ))
}

fn is_query_file(name: &str) -> bool {
    name.ends_with(".json")
        && std::iter::once(QUERY_PREFIX)
            .chain(OLD_QUERY_PREFIXES.iter().copied())
            .any(|p| name.starts_with(p))
}

pub fn save_last(response: &SearchResponse) {
    let dir = cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(json) = serde_json::to_string(response) {
        let _ = std::fs::write(last_path(), &json);
    }
}

pub fn load_last() -> Option<SearchResponse> {
    let content = std::fs::read_to_string(last_path()).ok()?;
    serde_json::from_str(&content).ok()
}

/// Age of the `--last` snapshot from the file's mtime (the snapshot itself
/// predates timestamped entries), so replays can report how stale they are.
pub fn last_age_secs() -> Option<u64> {
    let modified = std::fs::metadata(last_path()).ok()?.modified().ok()?;
    SystemTime::now()
        .duration_since(modified)
        .ok()
        .map(|d| d.as_secs())
}

/// Delete every cached entry (query cache + --last snapshot). Returns the
/// number of files removed.
pub fn clear() -> usize {
    let Ok(entries) = std::fs::read_dir(cache_dir()) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if (is_query_file(&name) || name == "last.json")
            && std::fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

#[derive(Serialize, Deserialize)]
struct CachedEntry {
    timestamp: u64,
    /// The exact query and mode, compared on load so a hash collision can
    /// never serve another query's results.
    #[serde(default)]
    query: String,
    #[serde(default)]
    mode: String,
    /// Requested count at cache time. An entry only satisfies a request for the
    /// same or fewer results — otherwise `-c 20` would silently get a cached
    /// 5-result response.
    #[serde(default)]
    count: usize,
    response: SearchResponse,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Save a query result to the TTL cache. Callers should skip empty results.
pub fn save_query(query: &str, mode: &str, count: usize, response: &SearchResponse) {
    save_query_in(&cache_dir(), now_secs(), query, mode, count, response);
}

/// Load a cached query result if fresh and it held at least `count` results.
/// Returns the response plus its age in seconds so the caller can mark the
/// replay as cached in the envelope.
pub fn load_query(query: &str, mode: &str, count: usize) -> Option<(SearchResponse, u64)> {
    load_query_in(&cache_dir(), now_secs(), query, mode, count)
}

fn save_query_in(
    dir: &Path,
    now: u64,
    query: &str,
    mode: &str,
    count: usize,
    response: &SearchResponse,
) {
    let _ = std::fs::create_dir_all(dir);
    sweep_expired(dir, now);
    let entry = CachedEntry {
        timestamp: now,
        query: query.to_string(),
        mode: mode.to_string(),
        count,
        response: response.clone(),
    };
    if let Ok(json) = serde_json::to_string(&entry) {
        let _ = std::fs::write(query_cache_path(dir, query, mode), json);
    }
}

fn load_query_in(
    dir: &Path,
    now: u64,
    query: &str,
    mode: &str,
    count: usize,
) -> Option<(SearchResponse, u64)> {
    let path = query_cache_path(dir, query, mode);
    let content = std::fs::read_to_string(&path).ok()?;
    let entry: CachedEntry = serde_json::from_str(&content).ok()?;
    if entry.query != query || entry.mode != mode {
        return None;
    }
    // saturating_sub: a backward clock step (NTP/VM resume) must not underflow.
    let age = now.saturating_sub(entry.timestamp);
    if age >= CACHE_TTL_SECS {
        let _ = std::fs::remove_file(&path);
        return None;
    }
    if entry.count < count {
        return None;
    }
    let mut resp = entry.response;
    resp.results.truncate(count);
    resp.metadata.result_count = resp.results.len();
    Some((resp, age))
}

/// Remove query-cache files older than the TTL (by mtime), so entries that
/// are never asked for again do not accumulate.
fn sweep_expired(dir: &Path, now: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_query_file(&name) {
            continue;
        }
        let written = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        if written.is_some_and(|w| now.saturating_sub(w) >= CACHE_TTL_SECS) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> Self {
            let d = std::env::temp_dir().join(format!(
                "search-cache-test-{tag}-{}-{}",
                std::process::id(),
                now_secs()
            ));
            std::fs::create_dir_all(&d).unwrap();
            Self(d)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn response(url: &str) -> SearchResponse {
        serde_json::from_value(serde_json::json!({
            "version": "1", "status": "success", "query": url, "mode": "extract",
            "results": [{"title": "t", "url": url, "snippet": "s", "source": "x"}],
            "metadata": {"elapsed_ms": 1, "result_count": 1,
                         "providers_queried": ["x"], "providers_failed": []}
        }))
        .unwrap()
    }

    #[test]
    fn url_queries_that_differ_in_case_do_not_share_an_entry() {
        let dir = TempDir::new("case");
        let now = now_secs();
        let upper = "https://x.test/Foo";
        save_query_in(&dir.0, now, upper, "extract", 1, &response(upper));
        assert!(load_query_in(&dir.0, now, upper, "extract", 1).is_some());
        assert!(load_query_in(&dir.0, now, "https://x.test/foo", "extract", 1).is_none());
    }

    #[test]
    fn expired_entries_are_removed_on_read() {
        let dir = TempDir::new("ttl");
        let now = now_secs();
        save_query_in(&dir.0, now, "q", "general", 1, &response("https://a.test/"));
        let later = now + CACHE_TTL_SECS;
        assert!(load_query_in(&dir.0, later, "q", "general", 1).is_none());
        assert!(!query_cache_path(&dir.0, "q", "general").exists());
    }

    #[test]
    fn save_sweeps_expired_files_of_every_generation() {
        let dir = TempDir::new("sweep");
        for name in ["q_old.json", "q2_old.json", "q3_old.json", "last.json"] {
            std::fs::write(dir.0.join(name), "{}").unwrap();
        }
        let later = now_secs() + CACHE_TTL_SECS + 1;
        save_query_in(
            &dir.0,
            later,
            "q",
            "general",
            1,
            &response("https://a.test/"),
        );
        let mut left: Vec<String> = std::fs::read_dir(&dir.0)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        left.sort();
        assert_eq!(left.len(), 2, "{left:?}");
        assert!(left.contains(&"last.json".to_string()));
    }
}
