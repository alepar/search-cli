//! Reddit through routes that never request reddit.com.
//!
//! Reddit refuses honest automated clients: www.reddit.com serves an empty
//! JS shell, `.json` and old.reddit answer a 403 block page, RSS ends on
//! 2026-11-13, and the Data API closed to new apps on 2026-10-31 (unregistered
//! apps are cut off 2027-01-12, everyone without a written agreement around
//! March 2027). After *Reddit v. SerpApi* (S.D.N.Y. 2026-07-31) let DMCA §1201
//! claims over evading its bot detection proceed, anything that disguises a
//! client to get past those blocks is out of bounds.
//!
//! What remains is search indexes that already hold Reddit pages. The reddit
//! mode fans a query out to those indexes, scoped to reddit.com; the extract
//! route recovers one thread's indexed copy from the Perplexity Search API by
//! post ID. Both say where the text came from, and neither falls back to
//! fetching reddit.com.
//!
//! `reddit.reader` departs from that, off unless the user sets it. It names
//! readers the extract route tries in order before the indexed copy. `jina`
//! and `kagi` ask a third party to render the live thread, which reddit.com's
//! terms do not sanction. `arctic_shift` reads an archive that collected
//! through the Data API. Every use logs a warning.

use crate::context::AppContext;
use crate::errors::SearchError;
use crate::providers::kagi::{self, Kagi};
use crate::providers::perplexity::{self, Perplexity};
use crate::providers::{arctic_shift, Provider};
use crate::types::{FailureCategory, SearchOpts, SearchResult};
use crate::walls::{self, Wall};
use serde_json::json;
use std::collections::HashSet;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use url::Url;

/// Domain scope the reddit mode forces on every provider.
pub const DOMAIN: &str = "reddit.com";

/// Page text requested per thread when extracting. The Search API returns
/// query-relevant excerpts, so this bounds the copy rather than filling it.
const EXTRACT_TOKENS_PER_PAGE: u32 = 4096;
/// Candidate pages examined for the target post ID.
const EXTRACT_CANDIDATES: usize = 10;
/// The whole route, inside the engine's 60 s limit. That limit cancels the
/// fetch and every failure recorded so far, so the route stops itself first.
const ROUTE_BUDGET: Duration = Duration::from_secs(58);
/// The readers' share of it; the rest is held back for the index fallback.
const READER_BUDGET: Duration = Duration::from_secs(45);
/// Jina Reader takes about 40 s to render a Reddit thread.
const JINA_DEADLINE: Duration = Duration::from_secs(45);
/// One Kagi Extract call: the API's own 10 s limit plus the round trip.
const KAGI_DEADLINE: Duration = Duration::from_secs(15);
/// Reddit blocks Kagi's fetcher now and then, and the next try usually passes.
const KAGI_ATTEMPTS: usize = 3;
const KAGI_RETRY_DELAY: Duration = Duration::from_secs(2);
/// Shown with every index copy while `reddit.reader` is unset, so an agent
/// can offer the opt-in. Setting any value silences it.
pub const READER_HINT: &str = "Index copies are rewritten excerpts. Full threads are an opt-in, \
     outside Reddit's terms: `search config set reddit.reader kagi,arctic_shift` tries a live \
     Kagi render, then the Arctic Shift archive (`jina` is also a reader); \
     `search config set reddit.reader index` keeps index copies and hides this hint";

/// A Reddit thread, identified by its base-36 post ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadRef {
    /// Subreddit name without `r/`, when the URL carried one.
    pub subreddit: Option<String>,
    /// Base-36 post ID, lowercase (the `t3_` fullname without its prefix).
    pub id: String,
    /// Title slug from the URL, when present.
    pub slug: Option<String>,
    /// Comment ID, for a comment permalink.
    pub comment: Option<String>,
}

impl ThreadRef {
    /// One URL per thread, whatever host or path variant it was found
    /// under, so fusion merges the same thread from different providers.
    pub fn canonical_url(&self) -> String {
        match &self.subreddit {
            Some(sub) => format!("https://www.reddit.com/r/{sub}/comments/{}/", self.id),
            None => format!("https://www.reddit.com/comments/{}/", self.id),
        }
    }

    /// Queries to find this thread in a search index, best first: the
    /// title slug, then the bare post ID (which also covers a truncated or
    /// edited slug), each with the subreddit.
    fn search_queries(&self) -> Vec<String> {
        let with_sub = |q: String| match &self.subreddit {
            Some(sub) => format!("{q} r/{sub}"),
            None => q,
        };
        let mut queries: Vec<String> = self
            .slug
            .iter()
            .map(|slug| with_sub(slug.replace('_', " ")))
            .collect();
        queries.push(with_sub(self.id.clone()));
        queries
    }
}

/// True for reddit.com, any of its subdomains (www, old, new, np, m, ...)
/// and the redd.it short-link host.
pub fn is_reddit_url(url: &Url) -> bool {
    url.host_str().is_some_and(|h| {
        let h = h.to_ascii_lowercase();
        h == "reddit.com" || h.ends_with(".reddit.com") || h == "redd.it"
    })
}

/// Parse a thread or comment permalink. Accepts `/r/<sub>/comments/<id>/...`,
/// `/user/<name>/comments/<id>/...`, `/comments/<id>/...` and `redd.it/<id>`.
/// Share links (`/r/<sub>/s/<token>`) only resolve through reddit.com and are
/// not threads here.
pub fn parse_thread(url: &Url) -> Option<ThreadRef> {
    if !is_reddit_url(url) {
        return None;
    }
    let segs: Vec<&str> = url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();

    if url
        .host_str()
        .is_some_and(|h| h.eq_ignore_ascii_case("redd.it"))
    {
        let id = segs.first().filter(|s| is_post_id(s))?;
        return Some(ThreadRef {
            subreddit: None,
            id: id.to_ascii_lowercase(),
            slug: None,
            comment: None,
        });
    }

    let (subreddit, rest) = match segs.as_slice() {
        ["r", sub, rest @ ..] => (Some(sub.to_string()), rest),
        ["user" | "u", _, rest @ ..] => (None, rest),
        rest => (None, rest),
    };
    let ["comments", id, tail @ ..] = rest else {
        return None;
    };
    if !is_post_id(id) {
        return None;
    }
    // Tail: [slug], [slug, comment_id], or ["comment", comment_id].
    let (slug, comment) = match tail {
        ["comment", c, ..] => (None, Some(*c)),
        [s, c, ..] => (Some(*s), Some(*c)),
        [s] => (Some(*s), None),
        _ => (None, None),
    };
    Some(ThreadRef {
        subreddit,
        id: id.to_ascii_lowercase(),
        slug: slug.filter(|s| !s.is_empty()).map(str::to_string),
        comment: comment
            .filter(|c| is_post_id(c))
            .map(|c| c.to_ascii_lowercase()),
    })
}

/// Base-36 IDs are short lowercase alphanumerics; anything else in that path
/// position is not a post (e.g. `/comments/` listing pages).
fn is_post_id(s: &str) -> bool {
    (1..=13).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Reddit-mode filter for one search result: keep threads only, rewrite the
/// URL to the canonical thread URL, and record subreddit/post ID (and the
/// comment, for a permalink) in `extra`. Subreddit, user and wiki pages are
/// dropped: the mode finds threads.
pub fn to_thread_result(mut r: SearchResult) -> Option<SearchResult> {
    let thread = Url::parse(&r.url).ok().as_ref().and_then(parse_thread)?;
    r.url = thread.canonical_url();
    let extra = r.extra.get_or_insert_with(|| json!({}));
    if let Some(obj) = extra.as_object_mut() {
        obj.insert("post_id".into(), json!(thread.id));
        if let Some(sub) = &thread.subreddit {
            obj.insert("subreddit".into(), json!(sub));
        }
        if let Some(c) = &thread.comment {
            obj.insert("comment_id".into(), json!(c));
        }
    }
    Some(r)
}

/// Options for a reddit-mode provider call: the domain allowlist is replaced
/// by reddit.com, so every provider searches Reddit only.
pub fn scoped_opts(opts: &SearchOpts) -> SearchOpts {
    SearchOpts {
        include_domains: vec![DOMAIN.to_string()],
        exclude_domains: Vec::new(),
        ..opts.clone()
    }
}

/// Reads one thread for extract/scrape. Provider name "reddit".
pub struct Reddit {
    ctx: Arc<AppContext>,
    billed: Mutex<Vec<String>>,
}

impl Reddit {
    pub const NAME: &'static str = "reddit";

    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self {
            ctx,
            billed: Mutex::new(Vec::new()),
        }
    }

    /// Paid calls made so far that the route's name does not cover.
    pub fn take_billed_calls(&self) -> Vec<String> {
        self.billed
            .lock()
            .map(|mut calls| std::mem::take(&mut *calls))
            .unwrap_or_default()
    }

    fn bill(&self, name: &str) {
        if let Ok(mut calls) = self.billed.lock() {
            calls.push(name.to_string());
        }
    }

    /// One thread for extract: the readers in `reddit.reader` in order, then
    /// its indexed copy, found in the Perplexity Search API by post ID. What
    /// went wrong with each reader stays with the result, or in the error if
    /// the index fails too. Errors with `no_sanctioned_route` when the URL is
    /// not a thread or nothing could be read.
    pub async fn fetch_thread(&self, url: &Url) -> Result<Vec<SearchResult>, SearchError> {
        let Some(thread) = parse_thread(url) else {
            return Err(no_route(
                "only thread URLs (/r/<sub>/comments/<id>/...) can be read; Reddit blocks \
                 automated fetches of other pages and offers no open API. Find threads with \
                 `search -m reddit -q <topic>`",
            ));
        };
        let started = Instant::now();
        let chain = ReaderChain::from_config(&self.ctx.config.reddit.reader);
        let (page, failures) = run_readers(&chain, started, READER_BUDGET, |reader| {
            let thread = &thread;
            async move { self.read(reader, thread, started).await }
        })
        .await;
        if let Some(mut page) = page {
            if !failures.is_empty() {
                set_extra(&mut page, "reader_errors", failures_json(&failures));
            }
            return Ok(vec![page]);
        }
        let left = ROUTE_BUDGET.saturating_sub(started.elapsed());
        let mut copy = tokio::time::timeout(left, self.index_copy(&thread))
            .await
            .unwrap_or_else(|_| Err(no_route("the index did not answer in time")))
            .map_err(|e| with_reader_errors(e, &failures))?;
        if !failures.is_empty() {
            set_extra(&mut copy, "reader_errors", failures_json(&failures));
        }
        if chain.hint {
            set_extra(&mut copy, "reader_hint", json!(READER_HINT));
        }
        Ok(vec![copy])
    }

    async fn read(
        &self,
        reader: Reader,
        thread: &ThreadRef,
        started: Instant,
    ) -> Result<SearchResult, String> {
        match reader {
            Reader::Jina => self.jina_render(thread).await,
            Reader::Kagi => self.kagi_render(thread).await,
            Reader::ArcticShift => self.arctic_copy(thread, started).await,
        }
    }

    /// The live thread through Jina Reader. A wall or an empty page counts
    /// as a failure, so the caller moves on.
    async fn jina_render(&self, thread: &ThreadRef) -> Result<SearchResult, String> {
        tracing::warn!(
            "reddit: reading the live thread through Jina Reader (reddit.reader); \
             Reddit's terms do not permit unauthorised automated access"
        );
        let jina = super::jina::Jina::new(self.ctx.clone());
        let pages = jina
            .read_url_within(&thread.canonical_url(), JINA_DEADLINE)
            .await
            .map_err(|e| e.to_string())?;
        let page = pages
            .into_iter()
            .next()
            .ok_or_else(|| "empty response".to_string())?;
        if page.snippet.trim().is_empty() {
            return Err("empty page".to_string());
        }
        if let Some(wall) = walls::classify_reader_text(&page.snippet) {
            return Err(format!("Jina Reader returned a {}", wall.code()));
        }
        Ok(live_result(thread, page.title, page.snippet, "jina_reader"))
    }

    /// The live thread through Kagi Extract. A block is retried, since the
    /// next try usually passes; any other failure moves on at once. Every
    /// call is billed, including a blocked one.
    async fn kagi_render(&self, thread: &ThreadRef) -> Result<SearchResult, String> {
        let kagi = Kagi::new(self.ctx.clone());
        if !kagi.is_configured() {
            return Err("no Kagi API key (keys.kagi or KAGI_API_KEY)".to_string());
        }
        tracing::warn!(
            "reddit: reading the live thread through Kagi Extract (reddit.reader); \
             Reddit's terms do not permit unauthorised automated access"
        );
        let url = thread.canonical_url();
        for attempt in 1..=KAGI_ATTEMPTS {
            if attempt > 1 {
                tokio::time::sleep(KAGI_RETRY_DELAY).await;
            }
            self.bill(kagi::EXTRACT_SOURCE);
            let markdown = kagi
                .extract_page(&url, KAGI_DEADLINE)
                .await
                .map_err(|e| e.to_string())?;
            match walls::classify_reader_text(&markdown) {
                None => {
                    let title = markdown_title(&markdown);
                    return Ok(live_result(thread, title, markdown, kagi::EXTRACT_SOURCE));
                }
                Some(wall) if is_block(wall) && attempt < KAGI_ATTEMPTS => {
                    tracing::warn!(
                        "reddit: Kagi Extract returned a {} (attempt {attempt} of \
                         {KAGI_ATTEMPTS})",
                        wall.code()
                    );
                }
                Some(wall) if is_block(wall) => {
                    return Err(format!(
                        "Kagi Extract returned a {} on all {KAGI_ATTEMPTS} attempts",
                        wall.code()
                    ));
                }
                Some(wall) => return Err(format!("Kagi Extract returned a {}", wall.code())),
            }
        }
        unreachable!("the last attempt returns")
    }

    /// The thread from the Arctic Shift archive. Structured records, not a
    /// page, so there is no wall to look for.
    async fn arctic_copy(
        &self,
        thread: &ThreadRef,
        started: Instant,
    ) -> Result<SearchResult, String> {
        tracing::warn!(
            "reddit: reading the thread from the Arctic Shift archive (reddit.reader); \
             Reddit has not licensed it, and it may hold text deleted from Reddit"
        );
        let end = started + READER_BUDGET;
        let archived = arctic_shift::fetch_thread(&self.ctx.client, &thread.id, end).await?;
        Ok(archive_result(thread, archived))
    }

    /// The thread's copy in the Perplexity Search API. Fails with `model_text`
    /// when the index only holds model-written copies, otherwise with
    /// `no_sanctioned_route`.
    async fn index_copy(&self, thread: &ThreadRef) -> Result<SearchResult, SearchError> {
        let pplx = Perplexity::new(self.ctx.clone());
        let opts = scoped_opts(&SearchOpts::default());
        let lookup = |query: String| {
            let (pplx, opts) = (&pplx, &opts);
            async move {
                pplx.search_index(&query, EXTRACT_CANDIDATES, opts, EXTRACT_TOKENS_PER_PAGE)
                    .await
            }
        };
        let mut rejected = HashSet::new();
        let mut copies = Vec::new();
        for query in thread.search_queries() {
            if let Some(copy) = pick_thread_copy(thread, lookup(query).await?, &mut rejected) {
                copies.push(copy);
                break;
            }
        }
        // Excerpts follow the query. Found by bare ID, the index picks text
        // that matches nothing, and a model's note shows only when the excerpt
        // reaches it, so always ask again with the title it reported.
        let title_query = copies.first().map(|(_, first)| {
            match first.extra.as_ref().and_then(|e| e["subreddit"].as_str()) {
                Some(sub) => format!("{} r/{sub}", first.title),
                None => first.title.clone(),
            }
        });
        if let Some(query) = title_query {
            let retry = lookup(query)
                .await
                .ok()
                .and_then(|pages| pick_thread_copy(thread, pages, &mut rejected));
            copies.extend(retry);
        }
        // Another excerpt of a rejected page is the same rewrite without its note.
        let copy = copies
            .into_iter()
            .filter(|(index_url, _)| !rejected.contains(index_url))
            .map(|(_, copy)| copy)
            .max_by_key(|copy| copy.snippet.len());
        match copy {
            Some(copy) => Ok(copy),
            None if !rejected.is_empty() => Err(SearchError::Blocked {
                provider: Reddit::NAME,
                code: Wall::ModelText.code(),
                message: format!(
                    "the index's copy of this thread is model-written text (a paraphrase or an \
                     \"unable to access\" note), not the thread. Open it in a browser: {}",
                    thread.canonical_url()
                ),
            }),
            None => Err(no_route(&format!(
                "no search index holds a copy of this thread, and Reddit blocks automated \
                 fetches. Open it in a browser: {}",
                thread.canonical_url()
            ))),
        }
    }
}

/// A reader `reddit.reader` can name. The index is not one: it always runs
/// last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reader {
    Jina,
    Kagi,
    ArcticShift,
}

impl Reader {
    fn name(self) -> &'static str {
        match self {
            Self::Jina => "jina",
            Self::Kagi => "kagi",
            Self::ArcticShift => arctic_shift::SOURCE,
        }
    }
}

/// `reddit.reader` parsed for a run. `config set` validates the value, but a
/// hand-edited file or the env override may not be valid: a name this build
/// does not know is reported, and the read goes on without it.
#[derive(Debug, PartialEq, Eq)]
struct ReaderChain {
    readers: Vec<Reader>,
    unknown: Vec<String>,
    /// Unset: carry the opt-in hint.
    hint: bool,
}

impl ReaderChain {
    fn from_config(value: &str) -> Self {
        let names = crate::config::reddit_reader_names(value);
        let mut chain = Self {
            readers: Vec::new(),
            unknown: Vec::new(),
            hint: names.is_empty(),
        };
        for name in names {
            match name {
                "index" => {}
                "jina" => chain.readers.push(Reader::Jina),
                "kagi" => chain.readers.push(Reader::Kagi),
                "arctic_shift" => chain.readers.push(Reader::ArcticShift),
                other => chain.unknown.push(other.to_string()),
            }
        }
        chain
    }
}

/// A reader that gave no thread, and why.
#[derive(Debug, PartialEq, Eq)]
struct ReaderFailure {
    reader: String,
    error: String,
}

/// Try each reader in turn until one returns a page, within `budget` of
/// `started`. Returns that page, if any, and every failure on the way to it.
async fn run_readers<F, Fut>(
    chain: &ReaderChain,
    started: Instant,
    budget: Duration,
    read: F,
) -> (Option<SearchResult>, Vec<ReaderFailure>)
where
    F: Fn(Reader) -> Fut,
    Fut: Future<Output = Result<SearchResult, String>>,
{
    let mut failures: Vec<ReaderFailure> = chain
        .unknown
        .iter()
        .map(|name| {
            tracing::warn!("reddit: unknown reader '{name}' in reddit.reader; skipped");
            ReaderFailure {
                reader: name.clone(),
                error: "not a reader this build knows".to_string(),
            }
        })
        .collect();
    for (i, reader) in chain.readers.iter().enumerate() {
        let left = budget.saturating_sub(started.elapsed());
        let outcome = if left.is_zero() {
            Err("skipped: the route's time budget is spent".to_string())
        } else {
            tokio::time::timeout(left, read(*reader))
                .await
                .unwrap_or_else(|_| Err(format!("timed out with {}s left", left.as_secs())))
        };
        match outcome {
            Ok(page) => return (Some(page), failures),
            Err(error) => {
                let next = chain
                    .readers
                    .get(i + 1)
                    .map_or("the index copy", |r| r.name());
                tracing::warn!("reddit: {} failed ({error}); trying {next}", reader.name());
                failures.push(ReaderFailure {
                    reader: reader.name().to_string(),
                    error,
                });
            }
        }
    }
    (None, failures)
}

fn failures_json(failures: &[ReaderFailure]) -> serde_json::Value {
    json!(failures
        .iter()
        .map(|f| json!({"reader": f.reader, "error": f.error}))
        .collect::<Vec<_>>())
}

/// A wall worth another try: it comes and goes with Reddit's bot detection.
fn is_block(wall: Wall) -> bool {
    matches!(wall, Wall::Bot | Wall::Login)
}

/// The first top-level heading.
fn markdown_title(markdown: &str) -> String {
    markdown
        .lines()
        .find_map(|l| l.strip_prefix("# "))
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn set_extra(r: &mut SearchResult, key: &str, value: serde_json::Value) {
    if let Some(obj) = r.extra.get_or_insert_with(|| json!({})).as_object_mut() {
        obj.insert(key.to_string(), value);
    }
}

/// A live render as a Reddit result. Collapsed replies are not expanded (Kagi
/// showed 92 comments where the archive holds 118), so it is not the complete
/// comment tree. The text is the thread's own words.
fn live_result(thread: &ThreadRef, title: String, text: String, via: &str) -> SearchResult {
    let mut extra = json!({
        "post_id": thread.id,
        "retrieved_via": via,
        "copy": "live_render",
        "complete": false,
        "verbatim": true,
    });
    if let Some(sub) = &thread.subreddit {
        extra["subreddit"] = json!(sub);
    }
    SearchResult {
        title,
        url: thread.canonical_url(),
        snippet: text,
        source: Reddit::NAME.to_string(),
        published: None,
        image_url: None,
        extra: Some(extra),
    }
}

/// An archived thread as a Reddit result. The maintainer puts comment coverage
/// at 90-95%, and the words are as captured.
fn archive_result(thread: &ThreadRef, archived: arctic_shift::Thread) -> SearchResult {
    let subreddit = thread.subreddit.clone().or(archived.subreddit);
    let canonical = ThreadRef {
        subreddit: subreddit.clone(),
        ..thread.clone()
    };
    let mut extra = json!({
        "post_id": thread.id,
        "retrieved_via": arctic_shift::SOURCE,
        "copy": "archive",
        "complete": false,
        "verbatim": true,
        "deleted_later_count": archived.deleted_later,
    });
    if let Some(sub) = subreddit {
        extra["subreddit"] = json!(sub);
    }
    if let Some(captured) = archived.captured_at {
        extra["captured_at"] = json!(captured);
    }
    SearchResult {
        title: archived.title,
        url: canonical.canonical_url(),
        snippet: archived.markdown,
        source: Reddit::NAME.to_string(),
        published: archived.published,
        image_url: None,
        extra: Some(extra),
    }
}

fn no_route(message: &str) -> SearchError {
    SearchError::Blocked {
        provider: Reddit::NAME,
        code: "no_sanctioned_route",
        message: message.to_string(),
    }
}

/// Keep every reader's failure in the error when the index fails too. An error
/// that carries no message of its own is rebuilt with the same code and
/// category, so a missing Perplexity key still reads as an auth problem.
fn with_reader_errors(e: SearchError, failures: &[ReaderFailure]) -> SearchError {
    if failures.is_empty() {
        return e;
    }
    let tried = failures
        .iter()
        .map(|f| format!("{}: {}", f.reader, f.error))
        .collect::<Vec<_>>()
        .join("; ");
    match e {
        SearchError::Blocked {
            provider,
            code,
            message,
        } => SearchError::Blocked {
            provider,
            code,
            message: format!("{message} (readers tried: {tried})"),
        },
        other => {
            let status = match other.category() {
                FailureCategory::Auth => Some(401),
                FailureCategory::RateLimit => Some(429),
                FailureCategory::Timeout => Some(408),
                _ => other.http_status(),
            };
            SearchError::Api {
                provider: Reddit::NAME,
                code: other.error_code(),
                message: format!("{other} (readers tried: {tried})"),
                status,
            }
        }
    }
}

/// From the index's candidate pages, the one for `thread` (longest excerpt
/// if the index returned several variants), relabelled as a Reddit result
/// that says it is an indexed copy rather than a live fetch, paired with the
/// URL the index holds it under. Pages that open in a model's voice go into
/// `rejected` and are skipped, along with any other excerpt of those URLs, in
/// favour of the remaining candidates.
fn pick_thread_copy(
    thread: &ThreadRef,
    pages: Vec<SearchResult>,
    rejected: &mut HashSet<String>,
) -> Option<(String, SearchResult)> {
    let pages: Vec<SearchResult> = pages
        .into_iter()
        .filter(|p| {
            Url::parse(&p.url)
                .ok()
                .as_ref()
                .and_then(parse_thread)
                .is_some_and(|t| t.id == thread.id)
        })
        .filter(|p| !p.snippet.trim().is_empty())
        .collect();
    rejected.extend(
        pages
            .iter()
            .filter(|p| walls::is_model_text(&p.snippet))
            .map(|p| p.url.clone()),
    );
    let page = pages
        .into_iter()
        .filter(|p| !rejected.contains(&p.url))
        .max_by_key(|p| p.snippet.len())?;

    // Prefer the subreddit the index saw when the input URL lacked one.
    let found = Url::parse(&page.url).ok().as_ref().and_then(parse_thread);
    let subreddit = thread
        .subreddit
        .clone()
        .or_else(|| found.and_then(|t| t.subreddit));
    let canonical = ThreadRef {
        subreddit: subreddit.clone(),
        ..thread.clone()
    };

    let mut extra = json!({
        "post_id": thread.id,
        "retrieved_via": perplexity::INDEX_SOURCE,
        "copy": "search_index",
        "complete": false,
        // The index rewrites what it holds: sampled phrases rarely match the
        // thread, so this is a paraphrase and not to be quoted.
        "verbatim": false,
    });
    if let Some(sub) = subreddit {
        extra["subreddit"] = json!(sub);
    }
    if let Some(updated) = page
        .extra
        .as_ref()
        .and_then(|e| e.get("index_last_updated"))
    {
        extra["index_last_updated"] = updated.clone();
    }
    let copy = SearchResult {
        title: page.title,
        url: canonical.canonical_url(),
        snippet: page.snippet,
        source: Reddit::NAME.to_string(),
        published: page.published,
        image_url: None,
        extra: Some(extra),
    };
    Some((page.url, copy))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn thread(url: &str) -> Option<ThreadRef> {
        parse_thread(&Url::parse(url).unwrap())
    }

    fn result(url: &str, snippet: &str) -> SearchResult {
        SearchResult {
            title: "t".into(),
            url: url.into(),
            snippet: snippet.into(),
            source: "x".into(),
            published: None,
            image_url: None,
            extra: None,
        }
    }

    #[test]
    fn recognises_reddit_hosts_only() {
        for u in [
            "https://www.reddit.com/r/rust/",
            "https://old.reddit.com/r/rust/",
            "https://np.reddit.com/",
            "https://reddit.com/",
            "https://redd.it/abc123",
        ] {
            assert!(is_reddit_url(&Url::parse(u).unwrap()), "{u}");
        }
        for u in [
            "https://notreddit.com/r/x",
            "https://reddit.com.evil.example/",
        ] {
            assert!(!is_reddit_url(&Url::parse(u).unwrap()), "{u}");
        }
    }

    #[test]
    fn parses_thread_variants_to_one_canonical_url() {
        let canon = "https://www.reddit.com/r/rust/comments/i5hppj/";
        for u in [
            "https://www.reddit.com/r/rust/comments/i5hppj/smol_vs_tokio_vs_asyncstd/",
            "https://old.reddit.com/r/rust/comments/i5hppj/smol_vs_tokio_vs_asyncstd",
            "http://np.reddit.com/r/rust/comments/i5hppj/?sort=top",
            "https://www.reddit.com/r/rust/comments/I5HPPJ/x/g0p7i4r/",
        ] {
            assert_eq!(thread(u).unwrap().canonical_url(), canon, "{u}");
        }
        let t = thread("https://www.reddit.com/r/rust/comments/i5hppj/smol_vs_tokio/").unwrap();
        assert_eq!(t.slug.as_deref(), Some("smol_vs_tokio"));
        assert_eq!(
            t.search_queries(),
            vec![
                "smol vs tokio r/rust".to_string(),
                "i5hppj r/rust".to_string()
            ]
        );
        let short = thread("https://redd.it/1jc6gis").unwrap();
        assert_eq!(short.search_queries(), vec!["1jc6gis".to_string()]);
    }

    #[test]
    fn parses_comment_permalinks() {
        let a = thread("https://www.reddit.com/r/rust/comments/i5hppj/smol/g0p7i4r/").unwrap();
        assert_eq!(a.comment.as_deref(), Some("g0p7i4r"));
        let b = thread("https://www.reddit.com/r/rust/comments/i5hppj/comment/g0p7i4r/").unwrap();
        assert_eq!(b.comment.as_deref(), Some("g0p7i4r"));
        assert_eq!(b.slug, None);
    }

    #[test]
    fn parses_short_and_subredditless_links() {
        let s = thread("https://redd.it/1jc6gis").unwrap();
        assert_eq!(s.id, "1jc6gis");
        assert_eq!(
            s.canonical_url(),
            "https://www.reddit.com/comments/1jc6gis/"
        );
        let u = thread("https://www.reddit.com/user/someone/comments/abc12/hello/").unwrap();
        assert_eq!(u.subreddit, None);
        assert_eq!(u.slug.as_deref(), Some("hello"));
        assert!(thread("https://www.reddit.com/comments/abc12/").is_some());
    }

    #[test]
    fn non_thread_pages_are_not_threads() {
        for u in [
            "https://www.reddit.com/r/rust/",
            "https://www.reddit.com/r/rust/top/?t=week",
            "https://www.reddit.com/r/rust/s/AbCdEf123",
            "https://www.reddit.com/user/someone/",
            "https://www.reddit.com/r/rust/wiki/faq",
            "https://www.reddit.com/r/rust/comments/",
            "https://example.com/r/rust/comments/i5hppj/x/",
        ] {
            assert!(thread(u).is_none(), "{u}");
        }
    }

    #[test]
    fn thread_results_are_canonicalised_and_others_dropped() {
        let r = to_thread_result(result(
            "https://old.reddit.com/r/rust/comments/i5hppj/smol/g0p7i4r/",
            "s",
        ))
        .unwrap();
        assert_eq!(r.url, "https://www.reddit.com/r/rust/comments/i5hppj/");
        let extra = r.extra.unwrap();
        assert_eq!(extra["post_id"], "i5hppj");
        assert_eq!(extra["subreddit"], "rust");
        assert_eq!(extra["comment_id"], "g0p7i4r");
        assert!(to_thread_result(result("https://www.reddit.com/r/rust/", "s")).is_none());
        assert!(to_thread_result(result("https://example.com/a", "s")).is_none());
    }

    #[test]
    fn scoped_opts_force_reddit_and_keep_other_filters() {
        let o = SearchOpts {
            include_domains: vec!["example.com".into()],
            exclude_domains: vec!["x.com".into()],
            freshness: Some("week".into()),
            ..Default::default()
        };
        let s = scoped_opts(&o);
        assert_eq!(s.include_domains, vec!["reddit.com".to_string()]);
        assert!(s.exclude_domains.is_empty());
        assert_eq!(s.freshness.as_deref(), Some("week"));
    }

    #[test]
    fn picks_the_matching_thread_copy_and_labels_it() {
        let t = thread("https://redd.it/1jc6gis").unwrap();
        let mut long = result(
            "https://www.reddit.com/r/rust/comments/1jc6gis/psa_asyncstd/",
            "### PSA\n### Comments\nUse smol. More text here.",
        );
        long.extra = Some(json!({"index_last_updated": "2025-11-03"}));
        let pages = vec![
            result(
                "https://www.reddit.com/r/rust/comments/other1/x/",
                "other thread",
            ),
            result(
                "https://www.reddit.com/r/rust/comments/1jc6gis/psa/",
                "short",
            ),
            long,
        ];
        let (index_url, copy) = pick_thread_copy(&t, pages, &mut HashSet::new()).unwrap();
        assert_eq!(
            index_url,
            "https://www.reddit.com/r/rust/comments/1jc6gis/psa_asyncstd/"
        );
        assert_eq!(copy.url, "https://www.reddit.com/r/rust/comments/1jc6gis/");
        assert_eq!(copy.source, "reddit");
        assert!(copy.snippet.contains("### Comments"));
        let extra = copy.extra.unwrap();
        assert_eq!(extra["copy"], "search_index");
        assert_eq!(extra["complete"], false);
        assert_eq!(extra["verbatim"], false);
        assert_eq!(extra["retrieved_via"], "perplexity_search");
        assert_eq!(extra["subreddit"], "rust");
        assert_eq!(extra["index_last_updated"], "2025-11-03");
    }

    fn chain(readers: &[Reader]) -> ReaderChain {
        ReaderChain {
            readers: readers.to_vec(),
            unknown: Vec::new(),
            hint: false,
        }
    }

    #[test]
    fn reader_config_values() {
        let hinted = ReaderChain::from_config("");
        assert!(hinted.readers.is_empty() && hinted.hint);
        assert!(ReaderChain::from_config("  ").hint);
        let index = ReaderChain::from_config("index");
        assert!(index.readers.is_empty() && !index.hint);
        assert_eq!(ReaderChain::from_config("jina").readers, vec![Reader::Jina]);
        assert_eq!(
            ReaderChain::from_config("kagi, arctic_shift,index").readers,
            vec![Reader::Kagi, Reader::ArcticShift]
        );
        let bad = ReaderChain::from_config("bing,kagi");
        assert_eq!(bad.readers, vec![Reader::Kagi]);
        assert_eq!(bad.unknown, vec!["bing".to_string()]);
    }

    #[tokio::test]
    async fn the_first_reader_to_succeed_wins_and_keeps_earlier_failures() {
        let c = chain(&[Reader::Kagi, Reader::ArcticShift, Reader::Jina]);
        let tried = Mutex::new(Vec::new());
        let (page, failures) = run_readers(&c, Instant::now(), READER_BUDGET, |reader| {
            tried.lock().unwrap().push(reader);
            async move {
                match reader {
                    Reader::Kagi => Err("blocked".to_string()),
                    _ => Ok(result("https://a.test", "page")),
                }
            }
        })
        .await;
        assert_eq!(page.unwrap().snippet, "page");
        assert_eq!(
            *tried.lock().unwrap(),
            vec![Reader::Kagi, Reader::ArcticShift]
        );
        assert_eq!(
            failures,
            vec![ReaderFailure {
                reader: "kagi".into(),
                error: "blocked".into()
            }]
        );
    }

    #[tokio::test]
    async fn every_failure_is_kept_when_no_reader_succeeds() {
        let mut c = chain(&[Reader::Kagi, Reader::ArcticShift]);
        c.unknown.push("bing".into());
        let (page, failures) =
            run_readers(&c, Instant::now(), READER_BUDGET, |reader| async move {
                Err(format!("{} failed", reader.name()))
            })
            .await;
        assert!(page.is_none());
        let names: Vec<_> = failures.iter().map(|f| f.reader.as_str()).collect();
        assert_eq!(names, ["bing", "kagi", "arctic_shift"]);
        assert_eq!(failures[2].error, "arctic_shift failed");
    }

    #[tokio::test]
    async fn a_slow_reader_is_cut_off_with_a_reason_and_later_ones_are_skipped() {
        let c = chain(&[Reader::Kagi, Reader::ArcticShift]);
        let (page, failures) =
            run_readers(&c, Instant::now(), Duration::from_millis(20), |_| async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(result("https://a.test", "late"))
            })
            .await;
        assert!(page.is_none());
        assert!(failures[0].error.contains("timed out"), "{failures:?}");
        assert!(failures[1].error.contains("skipped"), "{failures:?}");
    }

    #[test]
    fn budgets_stay_inside_the_engine_limit() {
        assert!(ROUTE_BUDGET < Duration::from_secs(60));
        assert!(READER_BUDGET >= JINA_DEADLINE && READER_BUDGET < ROUTE_BUDGET);
    }

    #[test]
    fn live_result_is_labelled_as_a_render() {
        let t = thread("https://old.reddit.com/r/rust/comments/i5hppj/smol/").unwrap();
        let r = live_result(&t, "T".into(), "# T\nComments".into(), "kagi_extract");
        assert_eq!(r.url, "https://www.reddit.com/r/rust/comments/i5hppj/");
        assert_eq!(r.source, "reddit");
        let extra = r.extra.unwrap();
        assert_eq!(extra["copy"], "live_render");
        assert_eq!(extra["retrieved_via"], "kagi_extract");
        assert_eq!(extra["subreddit"], "rust");
        assert_eq!(extra["complete"], false);
        assert_eq!(extra["verbatim"], true);
    }

    #[test]
    fn archive_result_is_labelled_and_fills_in_the_subreddit() {
        let t = thread("https://redd.it/1vvsc9y").unwrap();
        let r = archive_result(
            &t,
            arctic_shift::Thread {
                title: "T".into(),
                markdown: "# T".into(),
                subreddit: Some("gpu".into()),
                published: Some("2026-01-01T00:00:00Z".into()),
                captured_at: Some("2026-01-02T00:00:00Z".into()),
                deleted_later: 2,
            },
        );
        assert_eq!(r.url, "https://www.reddit.com/r/gpu/comments/1vvsc9y/");
        let extra = r.extra.unwrap();
        assert_eq!(extra["copy"], "archive");
        assert_eq!(extra["retrieved_via"], "arctic_shift");
        assert_eq!(extra["complete"], false);
        assert_eq!(extra["verbatim"], true);
        assert_eq!(extra["captured_at"], "2026-01-02T00:00:00Z");
        assert_eq!(extra["deleted_later_count"], 2);
    }

    #[test]
    fn title_is_the_first_top_level_heading() {
        assert_eq!(markdown_title("intro\n# RTX pro\n\n## Comments"), "RTX pro");
        assert_eq!(markdown_title("no heading"), "");
    }

    #[test]
    fn block_pages_from_kagi_are_walls_and_threads_are_not() {
        // The page Kagi returned on the first try at 1v56gxo.
        let page = "You've been blocked by network security.\n\nTo continue, log in to your \
                    Reddit account or use your developer token\n\nIf you think you've been \
                    blocked by mistake, file a ticket below and we'll look into it.\n\n\
                    [Log in](https://www.reddit.com/login/) [File a ticket](https://x.test/n)";
        assert!(walls::classify_reader_text(page).is_some_and(is_block));
        let thread = "# RTX\n\nr/gpu · u/a\n\nBought a card and it failed within a month.\n\n\
                      ## Comments\n\n- **u/b**\n  Same here, the PSU is fine.\n";
        assert_eq!(walls::classify_reader_text(thread), None);
    }

    #[test]
    fn no_copy_when_the_index_lacks_the_thread() {
        let t = thread("https://www.reddit.com/r/rust/comments/1jc6gis/psa/").unwrap();
        let pages = vec![
            result("https://www.reddit.com/r/rust/comments/other1/x/", "other"),
            result("https://www.reddit.com/r/rust/comments/1jc6gis/psa/", "  "),
        ];
        let mut rejected = HashSet::new();
        assert!(pick_thread_copy(&t, pages, &mut rejected).is_none());
        assert!(rejected.is_empty());
    }

    #[test]
    fn model_written_copies_are_skipped_for_the_remaining_candidates() {
        let t = thread("https://www.reddit.com/r/gpu/comments/1v4un5x/rtx/").unwrap();
        let note = format!(
            "### RTX\nI'm unable to access the original post, but here's a rewritten version \
             based on the provided content:\n---\n{}",
            "I bought a card. ".repeat(20)
        );
        let pages = vec![
            result("https://www.reddit.com/r/gpu/comments/1v4un5x/rtx/", &note),
            result(
                "https://old.reddit.com/r/gpu/comments/1v4un5x/rtx/",
                "### RTX\nBuy 1 card, brand new.",
            ),
        ];
        let mut rejected = HashSet::new();
        let (_, copy) = pick_thread_copy(&t, pages, &mut rejected).unwrap();
        assert!(copy.snippet.contains("Buy 1 card"));
        assert_eq!(rejected.len(), 1);

        // Another excerpt of the rejected page, with its note cut, stays out.
        let excerpt = vec![result(
            "https://www.reddit.com/r/gpu/comments/1v4un5x/rtx/",
            "### RTX\n...\n---\nI bought a card.",
        )];
        assert!(pick_thread_copy(&t, excerpt, &mut rejected).is_none());
    }

    #[test]
    fn reader_failures_stay_in_the_final_error() {
        let failures = [
            ReaderFailure {
                reader: "kagi".into(),
                error: "no Kagi API key".into(),
            },
            ReaderFailure {
                reader: "arctic_shift".into(),
                error: "HTTP 500".into(),
            },
        ];
        let e = with_reader_errors(no_route("no copy"), &failures);
        assert_eq!(e.error_code(), "no_sanctioned_route");
        assert_eq!(
            e.to_string(),
            "reddit: no copy (readers tried: kagi: no Kagi API key; arctic_shift: HTTP 500)"
        );
    }

    #[test]
    fn a_missing_index_key_keeps_its_category_and_the_reader_failures() {
        let failures = [ReaderFailure {
            reader: "kagi".into(),
            error: "blocked".into(),
        }];
        let missing = SearchError::AuthMissing {
            provider: "perplexity",
        };
        let e = with_reader_errors(missing, &failures);
        assert_eq!(e.error_code(), "auth_missing");
        assert_eq!(e.category(), FailureCategory::Auth);
        assert!(e.to_string().contains("kagi: blocked"), "{e}");
        assert!(e.to_string().contains("perplexity"), "{e}");
        let untouched = with_reader_errors(no_route("no copy"), &[]);
        assert_eq!(untouched.to_string(), "reddit: no copy");
    }
}
