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
//! One opt-in departs from that: with `reddit.reader = "jina"` the extract
//! route asks Jina Reader to render the live thread, which reddit.com's terms
//! do not sanction, and falls back to the indexed copy when that fails. It is
//! off unless the user sets it, and every use logs a warning.

use crate::context::AppContext;
use crate::errors::SearchError;
use crate::providers::perplexity::{self, Perplexity};
use crate::types::{SearchOpts, SearchResult};
use crate::walls::{self, Wall};
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use url::Url;

/// Domain scope the reddit mode forces on every provider.
pub const DOMAIN: &str = "reddit.com";

/// Page text requested per thread when extracting. The Search API returns
/// query-relevant excerpts, so this bounds the copy rather than filling it.
const EXTRACT_TOKENS_PER_PAGE: u32 = 4096;
/// Candidate pages examined for the target post ID.
const EXTRACT_CANDIDATES: usize = 10;
/// Jina Reader takes about 40 s to render a Reddit thread. The route's own
/// 60 s budget leaves room for the index fallback after this.
const JINA_DEADLINE: Duration = Duration::from_secs(45);
/// Shown with every index copy while `reddit.reader` is unset, so an agent
/// can offer the opt-in. Setting either value silences it.
pub const READER_HINT: &str = "Full live threads are available through Jina Reader as an opt-in \
     (outside Reddit's terms): `search config set reddit.reader jina`; \
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
}

impl Reddit {
    pub const NAME: &'static str = "reddit";

    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }

    /// One thread for extract. By default its indexed copy, found in the
    /// Perplexity Search API by post ID. With `reddit.reader = "jina"`, the
    /// live thread rendered by Jina Reader first. Errors with
    /// `no_sanctioned_route` when the URL is not a thread or nothing could be
    /// read.
    pub async fn fetch_thread(&self, url: &Url) -> Result<Vec<SearchResult>, SearchError> {
        let Some(thread) = parse_thread(url) else {
            return Err(no_route(
                "only thread URLs (/r/<sub>/comments/<id>/...) can be read; Reddit blocks \
                 automated fetches of other pages and offers no open API. Find threads with \
                 `search -m reddit -q <topic>`",
            ));
        };
        match Reader::from_config(&self.ctx.config.reddit.reader) {
            Reader::Jina => {
                let live_error = match self.live_render(&thread).await {
                    Ok(page) => return Ok(vec![page]),
                    Err(e) => e,
                };
                tracing::warn!("reddit: Jina Reader failed ({live_error}); using the index copy");
                let mut copy = self
                    .index_copy(&thread)
                    .await
                    .map_err(|e| with_live_error(e, &live_error))?;
                set_extra(&mut copy, "live_render_error", json!(live_error));
                Ok(vec![copy])
            }
            Reader::Index { hint } => {
                let mut copy = self.index_copy(&thread).await?;
                if hint {
                    set_extra(&mut copy, "reader_hint", json!(READER_HINT));
                }
                Ok(vec![copy])
            }
        }
    }

    /// The live thread through Jina Reader. A wall or an empty page counts
    /// as a failure, so the caller falls back to the index.
    async fn live_render(&self, thread: &ThreadRef) -> Result<SearchResult, String> {
        tracing::warn!(
            "reddit: reading the live thread through Jina Reader (reddit.reader = jina); \
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
        Ok(live_result(thread, page))
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

/// What `reddit.reader` selects. Anything but "jina" reads the index; only an
/// unset value carries the opt-in hint.
#[derive(Debug, PartialEq, Eq)]
enum Reader {
    Index { hint: bool },
    Jina,
}

impl Reader {
    fn from_config(value: &str) -> Self {
        match value.trim() {
            "jina" => Self::Jina,
            "" => Self::Index { hint: true },
            _ => Self::Index { hint: false },
        }
    }
}

fn set_extra(r: &mut SearchResult, key: &str, value: serde_json::Value) {
    if let Some(obj) = r.extra.get_or_insert_with(|| json!({})).as_object_mut() {
        obj.insert(key.to_string(), value);
    }
}

/// A Jina-rendered page as a Reddit result. Collapsed replies ("More
/// replies") are not expanded, so it is not the complete comment tree.
fn live_result(thread: &ThreadRef, page: SearchResult) -> SearchResult {
    let mut extra = json!({
        "post_id": thread.id,
        "retrieved_via": "jina_reader",
        "copy": "live_render",
        "complete": false,
    });
    if let Some(sub) = &thread.subreddit {
        extra["subreddit"] = json!(sub);
    }
    SearchResult {
        title: page.title,
        url: thread.canonical_url(),
        snippet: page.snippet,
        source: Reddit::NAME.to_string(),
        published: None,
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

/// Keep the live render's failure in the error when the index fails too.
fn with_live_error(e: SearchError, live_error: &str) -> SearchError {
    match e {
        SearchError::Blocked {
            provider,
            code,
            message,
        } => SearchError::Blocked {
            provider,
            code,
            message: format!("{message} (live render: {live_error})"),
        },
        other => other,
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
        assert_eq!(extra["retrieved_via"], "perplexity_search");
        assert_eq!(extra["subreddit"], "rust");
        assert_eq!(extra["index_last_updated"], "2025-11-03");
    }

    #[test]
    fn reader_config_values() {
        assert_eq!(Reader::from_config(""), Reader::Index { hint: true });
        assert_eq!(Reader::from_config("  "), Reader::Index { hint: true });
        assert_eq!(Reader::from_config("index"), Reader::Index { hint: false });
        assert_eq!(Reader::from_config("jina"), Reader::Jina);
    }

    #[test]
    fn live_result_is_labelled_as_a_render() {
        let t = thread("https://old.reddit.com/r/rust/comments/i5hppj/smol/").unwrap();
        let r = live_result(&t, result("https://r.jina.ai/x", "# Title\nComments"));
        assert_eq!(r.url, "https://www.reddit.com/r/rust/comments/i5hppj/");
        assert_eq!(r.source, "reddit");
        let extra = r.extra.unwrap();
        assert_eq!(extra["copy"], "live_render");
        assert_eq!(extra["retrieved_via"], "jina_reader");
        assert_eq!(extra["subreddit"], "rust");
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
    fn live_failure_stays_in_the_final_error() {
        let e = with_live_error(no_route("no copy"), "Jina Reader returned a bot_wall");
        assert_eq!(e.error_code(), "no_sanctioned_route");
        assert_eq!(
            e.to_string(),
            "reddit: no copy (live render: Jina Reader returned a bot_wall)"
        );
    }
}
