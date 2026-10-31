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
    assert_eq!(output.status.code().unwrap(), 2); // config error
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
