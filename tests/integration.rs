use assert_cmd::Command;
use predicates::prelude::*;
use std::time::Instant;

/// The binary under test. Cache and logs go to a scratch dir under target/
/// (Linux XDG paths), so test searches never touch the developer's real cache
/// or daily logs; config still comes from the real config dir, which is where
/// the live tests find their API keys.
fn search_cmd() -> Command {
    let scratch = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("integration-xdg");
    let mut cmd = Command::cargo_bin("search").unwrap();
    cmd.env("XDG_CACHE_HOME", scratch.join("cache"))
        .env("XDG_DATA_HOME", scratch.join("data"));
    cmd
}

/// Providers with a key configured, per `search config check`. Under the test
/// harness stdout is a pipe, so the command emits its JSON envelope.
fn configured_providers() -> Vec<String> {
    let output = search_cmd().args(["config", "check"]).output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_default();
    json["configured"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Check if any search provider is configured. Stealth needs no key, so it
/// counts as configured in stealth builds, but it only scrapes URLs and cannot
/// serve the live search tests.
fn has_any_provider() -> bool {
    configured_providers().iter().any(|p| p != "stealth")
}

fn has_provider(name: &str) -> bool {
    configured_providers().iter().any(|p| p == name)
}

// =============================================================================
// CLI structure tests (no API keys needed)
// =============================================================================

#[test]
fn test_help_output() {
    search_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Aggregates 14 search providers"))
        .stdout(predicate::str::contains("brave"))
        .stdout(predicate::str::contains("serper"))
        .stdout(predicate::str::contains("exa"));
}

#[test]
fn test_version() {
    search_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("search 0."));
}

#[test]
fn test_search_help_shows_modes() {
    search_cmd()
        .args(["search", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("academic"))
        .stdout(predicate::str::contains("people"))
        .stdout(predicate::str::contains("scholar"))
        .stdout(predicate::str::contains("patents"));
}

#[test]
fn test_agent_info_json() {
    let output = search_cmd().arg("agent-info").output().unwrap();
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["name"], "search");
    assert!(json["modes"].as_array().unwrap().len() >= 13);
    assert!(json["providers"].as_array().unwrap().len() >= 5);
    assert_eq!(json["config"]["env_prefix"], "SEARCH_");
    assert_eq!(json["auto_json_when_piped"], true);
    assert!(json["command_schemas"].is_object());
    assert!(json["exit_codes"].is_object());
}

#[test]
fn test_providers_json() {
    let output = search_cmd().args(["providers", "--json"]).output().unwrap();
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "success");
    let providers = json["providers"].as_array().unwrap();
    let names: Vec<&str> = providers
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"brave"));
    assert!(names.contains(&"serper"));
    assert!(names.contains(&"exa"));
    assert!(names.contains(&"jina"));
    assert!(names.contains(&"firecrawl"));
    assert!(names.contains(&"tavily"));
}

#[test]
fn test_config_check() {
    search_cmd()
        .args(["config", "check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("brave"))
        .stdout(predicate::str::contains("serper"))
        .stdout(predicate::str::contains("exa"))
        .stdout(predicate::str::contains("jina"))
        .stdout(predicate::str::contains("firecrawl"))
        .stdout(predicate::str::contains("tavily"));
}

#[test]
fn test_config_show() {
    search_cmd()
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("timeout"))
        .stdout(predicate::str::contains("count"));
}

#[test]
fn test_config_show_json_includes_settings_email() {
    // settings.email (scholarly contact) is not a secret; JSON shows it.
    let output = search_cmd().args(["config", "show"]).output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["settings"]["email"].is_string());
}

#[test]
fn test_no_providers_error_json() {
    // Use a fake provider name to ensure no providers match
    let output = search_cmd()
        .args(["search", "-q", "test", "-p", "nonexistent", "--json"])
        .output()
        .unwrap();

    assert_ne!(output.status.code().unwrap(), 0);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let json: serde_json::Value = serde_json::from_str(stderr.trim()).unwrap();
    assert_eq!(json["status"], "error");
    assert!(json["error"]["code"].as_str().is_some());
    assert!(json["error"]["message"].as_str().is_some());
}

#[test]
fn test_exit_code_no_providers() {
    let output = search_cmd()
        .args(["search", "-q", "test", "-p", "nonexistent", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code().unwrap(), 3); // bad input: unknown -p
}

// =============================================================================
// Real API tests (require configured providers)
// =============================================================================

#[test]
fn test_real_general_search() {
    if !has_any_provider() {
        eprintln!("SKIP: no providers configured");
        return;
    }

    let start = Instant::now();
    let output = search_cmd()
        .args([
            "search",
            "-q",
            "Rust programming language",
            "--json",
            "-c",
            "5",
        ])
        .output()
        .unwrap();
    let elapsed = start.elapsed();

    assert!(
        output.status.success(),
        "search failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "success");
    assert_eq!(json["mode"], "general");

    let results = json["results"].as_array().unwrap();
    assert!(!results.is_empty(), "expected results, got 0");

    // Check result structure
    let first = &results[0];
    assert!(first["title"].as_str().is_some());
    assert!(first["url"].as_str().is_some());
    assert!(first["source"].as_str().is_some());

    // Metadata
    assert!(json["metadata"]["elapsed_ms"].as_u64().unwrap() > 0);
    assert!(!json["metadata"]["providers_queried"]
        .as_array()
        .unwrap()
        .is_empty());

    eprintln!(
        "  PASS general search: {} results in {}ms (wall: {}ms)",
        results.len(),
        json["metadata"]["elapsed_ms"],
        elapsed.as_millis()
    );
}

#[test]
fn test_real_news_search() {
    if !has_provider("serper") && !has_provider("brave") {
        eprintln!("SKIP: need serper or brave for news");
        return;
    }

    let output = search_cmd()
        .args([
            "search",
            "-q",
            "technology news",
            "-m",
            "news",
            "--json",
            "-c",
            "5",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["mode"], "news");
    let results = json["results"].as_array().unwrap();
    assert!(!results.is_empty(), "news search returned 0 results");

    eprintln!("  PASS news search: {} results", results.len());
}

#[test]
fn test_real_academic_search() {
    if !has_provider("exa") && !has_provider("serper") {
        eprintln!("SKIP: need exa or serper for academic");
        return;
    }

    let output = search_cmd()
        .args([
            "search",
            "-q",
            "CRISPR gene editing",
            "-m",
            "academic",
            "--json",
            "-c",
            "5",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["mode"], "academic");
    let results = json["results"].as_array().unwrap();
    assert!(!results.is_empty(), "academic search returned 0 results");

    eprintln!("  PASS academic search: {} results", results.len());
}

#[test]
fn test_real_provider_filter_single() {
    if !has_provider("exa") {
        eprintln!("SKIP: need exa");
        return;
    }

    let output = search_cmd()
        .args([
            "search",
            "-q",
            "artificial intelligence",
            "-p",
            "exa",
            "--json",
            "-c",
            "3",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let providers = json["metadata"]["providers_queried"].as_array().unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0], "exa");

    // All results should be from exa
    for r in json["results"].as_array().unwrap() {
        assert!(
            r["source"].as_str().unwrap().starts_with("exa"),
            "unexpected source: {}",
            r["source"]
        );
    }

    eprintln!(
        "  PASS provider filter (exa only): {} results",
        json["results"].as_array().unwrap().len()
    );
}

#[test]
fn test_real_kagi_general_and_news() {
    if !has_provider("kagi") {
        eprintln!("SKIP: need kagi");
        return;
    }

    for mode in ["general", "news"] {
        let output = search_cmd()
            .args([
                "search",
                "-q",
                "rust programming language",
                "-m",
                mode,
                "-p",
                "kagi",
                "--json",
                "-c",
                "3",
            ])
            .output()
            .unwrap();

        assert!(output.status.success(), "kagi {mode} search failed");
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let results = json["results"].as_array().unwrap();
        assert!(!results.is_empty(), "kagi {mode} returned 0 results");
        assert!(results.len() <= 3);
        for r in results {
            assert_eq!(r["source"], "kagi");
        }
        eprintln!("  PASS kagi {mode}: {} results", results.len());
    }
}

#[test]
fn test_real_reddit_mode() {
    if !has_provider("perplexity") {
        eprintln!("SKIP: need perplexity");
        return;
    }
    let output = search_cmd()
        .args([
            "search",
            "-q",
            "tokio vs smol",
            "-m",
            "reddit",
            "-p",
            "perplexity",
            "--json",
            "-c",
            "5",
            "--no-cache",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "reddit mode search failed");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["mode"], "reddit");
    let results = json["results"].as_array().unwrap();
    assert!(!results.is_empty(), "reddit mode returned 0 results");
    for r in results {
        let url = r["url"].as_str().unwrap();
        assert!(
            url.starts_with("https://www.reddit.com/") && url.contains("/comments/"),
            "not a canonical thread URL: {url}"
        );
        assert!(r["extra"]["post_id"].is_string());
    }
    eprintln!("  PASS reddit mode: {} threads", results.len());
}

#[test]
fn test_real_reddit_extract() {
    if !has_provider("perplexity") {
        eprintln!("SKIP: need perplexity");
        return;
    }
    let output = search_cmd()
        .env("SEARCH_REDDIT_READER", "index")
        .args([
            "search",
            "-q",
            "https://old.reddit.com/r/rust/comments/1jc6gis/psa_asyncstd_has_been_officially_discontinued_use/",
            "-m",
            "extract",
            "--json",
            "--no-cache",
        ])
        .output()
        .unwrap();
    assert!(output.status.success(), "reddit extract failed");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        json["metadata"]["providers_queried"],
        serde_json::json!(["reddit"])
    );
    let r = &json["results"][0];
    assert_eq!(r["source"], "reddit");
    assert_eq!(r["url"], "https://www.reddit.com/r/rust/comments/1jc6gis/");
    assert_eq!(r["extra"]["copy"], "search_index");
    assert!(r["snippet"].as_str().unwrap().len() > 200);
    eprintln!(
        "  PASS reddit extract: {} chars",
        r["snippet"].as_str().unwrap().len()
    );
}

/// A non-thread reddit.com URL fails with no_sanctioned_route and never
/// reaches the scraper chain. Makes no network call.
#[test]
fn test_reddit_extract_non_thread_has_no_route() {
    let output = search_cmd()
        .args([
            "search",
            "-q",
            "https://www.reddit.com/r/rust/",
            "-m",
            "extract",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    // Error envelopes go to stderr, after any log lines.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let start = stderr.find('{').expect("JSON error envelope on stderr");
    let json: serde_json::Value = serde_json::from_str(&stderr[start..]).unwrap();
    let failures = json["error"]["provider_failures"].as_array().unwrap();
    assert_eq!(failures.len(), 1, "scraper chain ran: {failures:?}");
    assert_eq!(failures[0]["provider"], "reddit");
    assert_eq!(failures[0]["code"], "no_sanctioned_route");
    assert_eq!(failures[0]["category"], "blocked");
}

#[test]
fn test_real_domain_filter() {
    if !has_provider("exa") {
        eprintln!("SKIP: need exa for domain filter");
        return;
    }

    let output = search_cmd()
        .args([
            "search",
            "-q",
            "machine learning",
            "-d",
            "arxiv.org",
            "-p",
            "exa",
            "--json",
            "-c",
            "5",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let results = json["results"].as_array().unwrap();
    assert!(!results.is_empty());

    // All results should be from arxiv.org
    for r in results {
        let url = r["url"].as_str().unwrap();
        assert!(
            url.contains("arxiv.org"),
            "expected arxiv.org URL, got: {}",
            url
        );
    }

    eprintln!(
        "  PASS domain filter (arxiv.org): {} results",
        results.len()
    );
}

#[test]
fn test_real_freshness_filter() {
    if !has_provider("serper") {
        eprintln!("SKIP: need serper for freshness test");
        return;
    }

    let output = search_cmd()
        .args([
            "search", "-q", "AI news", "-f", "day", "-p", "serper", "--json", "-c", "5",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let results = json["results"].as_array().unwrap();
    assert!(!results.is_empty(), "freshness filter returned 0 results");

    eprintln!("  PASS freshness filter (day): {} results", results.len());
}

#[test]
fn test_real_last_cache() {
    if !has_any_provider() {
        eprintln!("SKIP: no providers");
        return;
    }

    // Own cache dir: other live tests run in parallel and would overwrite
    // last.json between the search and the replay.
    let cache = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("last-cache-test");
    let cached_cmd = || {
        let mut cmd = search_cmd();
        cmd.env("XDG_CACHE_HOME", &cache);
        cmd
    };

    // First search to populate cache
    let output1 = cached_cmd()
        .args(["search", "-q", "cache test query", "--json", "-c", "3"])
        .output()
        .unwrap();
    assert!(output1.status.success());

    // Replay from cache
    let start = Instant::now();
    let output2 = cached_cmd().args(["--last", "--json"]).output().unwrap();
    let cache_elapsed = start.elapsed();

    assert!(output2.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output2.stdout).unwrap();
    assert_eq!(json["status"], "success");
    assert!(!json["results"].as_array().unwrap().is_empty());

    // Cache replay should be near-instant (< 100ms)
    assert!(
        cache_elapsed.as_millis() < 500,
        "cache replay took {}ms",
        cache_elapsed.as_millis()
    );

    eprintln!("  PASS cache replay: {}ms", cache_elapsed.as_millis());
}

// =============================================================================
// Performance benchmark
// =============================================================================

#[test]
fn test_performance_benchmark() {
    if !has_any_provider() {
        eprintln!("SKIP: no providers for benchmark");
        return;
    }

    let queries = [
        "rust programming",
        "machine learning",
        "quantum computing",
        "climate change",
        "blockchain technology",
    ];

    let mut latencies = Vec::new();

    for q in &queries {
        let start = Instant::now();
        let output = search_cmd()
            .args(["search", "-q", q, "--json", "-c", "5"])
            .output()
            .unwrap();
        let elapsed = start.elapsed().as_millis();

        if output.status.success() {
            let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            let api_ms = json["metadata"]["elapsed_ms"].as_u64().unwrap_or(0);
            let count = json["results"].as_array().map(|a| a.len()).unwrap_or(0);
            eprintln!(
                "  query={:<25} results={:<3} api={}ms  wall={}ms",
                q, count, api_ms, elapsed
            );
            latencies.push(elapsed);
        } else {
            eprintln!("  query={:<25} FAILED", q);
        }
    }

    if latencies.is_empty() {
        eprintln!("  No successful queries for benchmark");
        return;
    }

    latencies.sort();
    let avg = latencies.iter().sum::<u128>() / latencies.len() as u128;
    let p50 = latencies[latencies.len() / 2];
    let p95 = latencies[latencies.len() * 95 / 100];
    let min = latencies[0];
    let max = latencies[latencies.len() - 1];

    eprintln!("\n  BENCHMARK ({} queries):", latencies.len());
    eprintln!("    avg:  {}ms", avg);
    eprintln!("    p50:  {}ms", p50);
    eprintln!("    p95:  {}ms", p95);
    eprintln!("    min:  {}ms", min);
    eprintln!("    max:  {}ms", max);
}

#[test]
fn test_agent_info_describes_collection_and_count_clamping() {
    let output = search_cmd().args(["agent-info"]).output().unwrap();
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("Fused modes wait for every provider"));
    assert!(text.contains("per-provider timeout"));
    assert!(text.contains("clamped to that provider's per-request maximum"));
    assert!(!text.contains("early-stop grace"));
    assert!(!text.contains("1.5s grace"));
}

// =============================================================================
// Provider signal log: mark-relevant, result rows, stats
// =============================================================================

/// A binary under test with its own empty data and cache dirs, for tests that
/// assert on log contents. The real config dir still supplies API keys.
struct LogSandbox {
    root: std::path::PathBuf,
}

impl LogSandbox {
    fn new(name: &str) -> Self {
        let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("log-sandbox")
            .join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::cargo_bin("search").unwrap();
        cmd.env("XDG_CACHE_HOME", self.root.join("cache"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env_remove("SEARCH_LOG");
        // macOS ignores XDG and derives every app dir from HOME.
        if cfg!(target_os = "macos") {
            cmd.env("HOME", &self.root);
        }
        cmd
    }

    fn log_dir(&self) -> std::path::PathBuf {
        if cfg!(target_os = "macos") {
            self.root.join("Library/Application Support/search/logs")
        } else {
            self.root.join("data/search/logs")
        }
    }

    /// Rows across every log file of one kind.
    fn rows(&self, kind: &str) -> Vec<serde_json::Value> {
        let Ok(dir) = std::fs::read_dir(self.log_dir()) else {
            return Vec::new();
        };
        dir.flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(kind))
            .flat_map(|e| {
                let text = std::fs::read_to_string(e.path()).unwrap();
                text.lines()
                    .map(|l| serde_json::from_str(l).expect("every log line is JSON"))
                    .collect::<Vec<serde_json::Value>>()
            })
            .collect()
    }

    fn seed(&self, file: &str, lines: &[serde_json::Value]) {
        std::fs::create_dir_all(self.log_dir()).unwrap();
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(self.log_dir().join(file), text).unwrap();
    }
}

fn stdout_json(cmd: &mut Command) -> serde_json::Value {
    let output = cmd.arg("--json").output().unwrap();
    serde_json::from_slice(&output.stdout).unwrap()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn mark_relevant_appends_one_row_per_url() {
    let sb = LogSandbox::new("mark");
    let out = stdout_json(
        sb.cmd()
            .args(["mark-relevant", "https://a.test/x", "https://b.test/y"]),
    );
    assert_eq!(out["status"], "success");
    assert_eq!(out["recorded"], 2);
    let rows = sb.rows("marks_");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1]["url"], "https://b.test/y");
    assert!(rows[0]["ts"].is_u64());
}

#[test]
fn mark_relevant_rejects_non_web_urls_and_records_nothing() {
    let sb = LogSandbox::new("mark-bad");
    sb.cmd()
        .args(["mark-relevant", "https://a.test", "file:///etc/passwd"])
        .assert()
        .code(3);
    sb.cmd()
        .args(["mark-relevant", "not a url"])
        .assert()
        .code(3);
    sb.cmd().args(["mark-relevant"]).assert().code(3);
    assert!(sb.rows("marks_").is_empty());
}

#[test]
fn search_log_off_disables_marks() {
    let sb = LogSandbox::new("mark-off");
    let out = stdout_json(
        sb.cmd()
            .env("SEARCH_LOG", "off")
            .args(["mark-relevant", "https://a.test"]),
    );
    assert_eq!(out["status"], "success");
    assert_eq!(out["recorded"], 0);
    assert!(!sb.log_dir().exists());
}

#[test]
fn concurrent_marks_stay_whole_lines() {
    let sb = LogSandbox::new("mark-concurrent");
    let children: Vec<_> = (0..16)
        .map(|i| {
            let mut cmd = std::process::Command::new(assert_cmd::cargo::cargo_bin("search"));
            cmd.env("XDG_CACHE_HOME", sb.root.join("cache"))
                .env("XDG_DATA_HOME", sb.root.join("data"))
                .env_remove("SEARCH_LOG")
                .stdout(std::process::Stdio::null())
                .arg("--json")
                .arg("mark-relevant");
            if cfg!(target_os = "macos") {
                cmd.env("HOME", &sb.root);
            }
            for n in 0..20 {
                cmd.arg(format!("https://a.test/{i}/{n}"));
            }
            cmd.spawn().unwrap()
        })
        .collect();
    for mut child in children {
        assert!(child.wait().unwrap().success());
    }
    assert_eq!(sb.rows("marks_").len(), 16 * 20);
}

#[test]
fn stats_reports_provider_signal() {
    let sb = LogSandbox::new("stats");
    let ts = now_secs() - 3600;
    let row = |provider: &str, url: &str| {
        serde_json::json!({"v": 1, "ts": ts, "search_id": "s1", "mode": "general",
                           "provider": provider, "rank": 1, "url": url})
    };
    let today = "2099-01-01"; // Future-dated names are never skipped as old.
    sb.seed(
        &format!("results_{today}.jsonl"),
        &[
            row("brave", "https://shared.test/a"),
            row("exa", "https://shared.test/a?utm_source=x"),
            row("exa", "https://exa-only.test/b"),
            row("serper", "https://serper-only.test/c"),
        ],
    );
    sb.seed(
        &format!("searches_{today}.jsonl"),
        &[
            serde_json::json!({"v": 1, "ts": ts, "search_id": "s1", "query": "q",
                "mode": "general", "providers_queried": ["brave", "exa", "serper"]}),
            serde_json::json!({"v": 1, "ts": ts + 1, "query": "https://serper-only.test/c",
                "mode": "extract", "result_count": 1, "providers_queried": ["jina"]}),
        ],
    );
    sb.cmd()
        .args([
            "mark-relevant",
            "https://www.shared.test/a#intro",
            "https://exa-only.test/b",
            "https://elsewhere.test/d",
        ])
        .assert()
        .success();

    let out = stdout_json(sb.cmd().arg("stats"));
    let signal = &out["stats"]["provider_signal"];
    assert_eq!(signal["marked_urls"], 3);
    assert_eq!(signal["marked_unattributed"], 1);
    let exa = &signal["providers"]["exa"];
    assert_eq!(exa["results"], 2);
    assert_eq!(exa["unique"], 1);
    assert_eq!(exa["relevant"], 2);
    assert_eq!(exa["unique_relevant"], 1);
    assert_eq!(exa["precision"], 1.0);
    assert_eq!(exa["cost_per_relevant_usd"], 0.0035);
    let brave = &signal["providers"]["brave"];
    assert_eq!(
        (&brave["relevant"], &brave["unique_relevant"]),
        (&1.into(), &0.into())
    );
    assert!(brave["cost_per_unique_relevant_usd"].is_null());
    let serper = &signal["providers"]["serper"];
    assert_eq!(
        (&serper["relevant"], &serper["extracted"]),
        (&0.into(), &1.into())
    );
}

#[test]
fn prune_covers_every_log_kind() {
    let sb = LogSandbox::new("prune");
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(40 * 86400);
    let line = [serde_json::json!({"ts": 1})];
    for name in [
        "searches_2020-01-01.jsonl",
        "results_2020-01-01.jsonl",
        "marks_2020-01-01.jsonl",
    ] {
        sb.seed(name, &line);
        let file = std::fs::File::options()
            .write(true)
            .open(sb.log_dir().join(name))
            .unwrap();
        file.set_modified(old).unwrap();
    }
    sb.seed("marks_2099-01-01.jsonl", &line);
    let out = stdout_json(sb.cmd().args(["stats", "--prune", "30"]));
    assert_eq!(out["pruned_files"], 3);
    assert!(sb.log_dir().join("marks_2099-01-01.jsonl").exists());
}

#[test]
fn agent_info_lists_mark_relevant() {
    let out = stdout_json(search_cmd().arg("agent-info"));
    let commands = out["commands"].as_array().unwrap();
    assert!(commands.iter().any(|c| c == "mark-relevant"));
    assert!(out["command_schemas"]["mark-relevant"].is_object());
}

// One billed Serper call per fresh run; patents is Serper-only and cacheable.
#[test]
fn fresh_search_logs_rows_and_cached_replay_logs_none() {
    if !has_provider("serper") {
        return;
    }
    let sb = LogSandbox::new("live-cache");
    let search = |sb: &LogSandbox| {
        stdout_json(
            sb.cmd()
                .args([
                    "-m",
                    "patents",
                    "-c",
                    "3",
                    "-q",
                    "solid state battery electrolyte",
                ])
                .arg("--quiet"),
        )
    };
    let first = search(&sb);
    assert_eq!(first["status"], "success", "{first}");
    let rows = sb.rows("results_").len();
    assert!(rows > 0);
    assert!(sb
        .rows("results_")
        .iter()
        .all(|r| r["provider"] == "serper"));

    let replay = search(&sb);
    assert_eq!(replay["metadata"]["cached"], true);
    assert_eq!(sb.rows("results_").len(), rows);
    assert_eq!(sb.rows("searches_").len(), 2);
}

#[test]
fn search_log_off_writes_no_result_rows() {
    if !has_provider("serper") {
        return;
    }
    let sb = LogSandbox::new("live-off");
    sb.cmd()
        .env("SEARCH_LOG", "off")
        .args([
            "-m",
            "patents",
            "-c",
            "1",
            "-q",
            "perovskite tandem cell",
            "--json",
        ])
        .assert()
        .success();
    assert!(!sb.log_dir().exists());
}
