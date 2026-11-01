//! Arctic Shift, a free archive of Reddit posts and comments. Used only by the
//! Reddit extract route, as the opt-in reader `arctic_shift`. It answers from
//! its own store, so this path never requests reddit.com.
//!
//! Comments are 90-95% complete by the maintainer's estimate, and collection
//! depends on the Data API that Reddit closes in 2027, so a thread may be
//! partial or stale.

use crate::dates;
use serde_json::Value;
use std::time::{Duration, Instant};

const BASE_URL: &str = "https://arctic-shift.photon-reddit.com";
/// Value of `extra.retrieved_via`.
pub const SOURCE: &str = "arctic_shift";
/// The tree endpoint collapses to `kind: "more"` past 50 comments by default;
/// its docs give this limit for a whole thread.
const TREE_LIMIT: u32 = 9999;
const REQUEST_DEADLINE: Duration = Duration::from_secs(20);
/// Shown in place of text the archive holds but its author or a moderator has
/// since removed from Reddit.
const DELETED_PLACEHOLDER: &str = "[deleted later]";
/// Whether that text is withheld. Reddit v. SerpApi treated getting around
/// Reddit's deletion feature as a harm, so the conservative default is yes.
const WITHHOLD_DELETED_TEXT: bool = true;

/// One archived thread, rendered in the same shape Kagi Extract returns.
#[derive(Debug, PartialEq)]
pub struct Thread {
    pub title: String,
    pub markdown: String,
    pub subreddit: Option<String>,
    pub published: Option<String>,
    /// Newest capture among the post and its comments.
    pub captured_at: Option<String>,
    /// Comments and post bodies withheld as deleted.
    pub deleted_later: usize,
}

/// Read a thread by post ID, finishing by `end`.
pub async fn fetch_thread(
    client: &reqwest::Client,
    id: &str,
    end: Instant,
) -> Result<Thread, String> {
    let mut waited = false;
    let posts = get_json(
        client,
        &format!("{BASE_URL}/api/posts/ids?ids={id}"),
        end,
        &mut waited,
    )
    .await?;
    let post = posts["data"]
        .get(0)
        .ok_or_else(|| "the archive holds no record of this post".to_string())?;
    let tree = get_json(
        client,
        &format!("{BASE_URL}/api/comments/tree?link_id=t3_{id}&limit={TREE_LIMIT}"),
        end,
        &mut waited,
    )
    .await?;
    let comments = tree["data"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    Ok(render(post, comments))
}

/// GET one API path as JSON. A 429 waits once per thread, when the reset fits
/// the time left. The response is parsed here so an unreadable tree (serde
/// stops at about 24 levels of replies) reports as this reader's failure.
async fn get_json(
    client: &reqwest::Client,
    url: &str,
    end: Instant,
    waited: &mut bool,
) -> Result<Value, String> {
    loop {
        let resp = client
            .get(url)
            .timeout(REQUEST_DEADLINE)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;
        let status = resp.status().as_u16();
        if status == 429 {
            let reset = resp
                .headers()
                .get("x-ratelimit-reset")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let left = end.saturating_duration_since(Instant::now());
            match rate_limit_wait(reset.as_deref(), left, *waited) {
                Some(wait) => {
                    *waited = true;
                    tracing::warn!("arctic_shift: rate limited, waiting {}s", wait.as_secs());
                    tokio::time::sleep(wait).await;
                    continue;
                }
                None => {
                    return Err(format!(
                        "rate limited (HTTP 429, resets in {}s)",
                        reset.as_deref().unwrap_or("?")
                    ));
                }
            }
        }
        if !resp.status().is_success() {
            return Err(format!("HTTP {status}"));
        }
        let text = resp
            .text()
            .await
            .map_err(|e| format!("reading the response failed: {e}"))?;
        let body: Value =
            serde_json::from_str(&text).map_err(|e| format!("unreadable response: {e}"))?;
        // Query timeouts come back as a 200 or 4xx with an `error` message.
        if let Some(error) = body.get("error").filter(|e| !e.is_null()) {
            return Err(format!("API error: {error}"));
        }
        return Ok(body);
    }
}

/// How long to wait on a 429: never twice, and only if the reset is known and
/// leaves time for the retry.
fn rate_limit_wait(reset: Option<&str>, left: Duration, waited: bool) -> Option<Duration> {
    if waited {
        return None;
    }
    let wait = Duration::from_secs(reset?.trim().parse().ok()?);
    (wait + Duration::from_secs(1) < left).then_some(wait)
}

/// Totals gathered while walking the record.
#[derive(Default)]
struct Walk {
    newest_capture: Option<u64>,
    deleted_later: usize,
}

impl Walk {
    fn capture(&mut self, record: &Value) {
        let first = record["retrieved_on"].as_u64();
        let second = record["_meta"]["retrieved_2nd_on"].as_u64();
        self.newest_capture = self.newest_capture.max(first).max(second);
    }

    /// The text to show for a record, withholding it when flagged as deleted.
    fn text<'a>(&mut self, record: &Value, text: &'a str) -> &'a str {
        if WITHHOLD_DELETED_TEXT && record["_meta"]["was_deleted_later"] == Value::Bool(true) {
            self.deleted_later += 1;
            DELETED_PLACEHOLDER
        } else {
            text
        }
    }
}

fn render(post: &Value, comments: &[Value]) -> Thread {
    let mut walk = Walk::default();
    walk.capture(post);

    let title = post["title"].as_str().unwrap_or_default().to_string();
    let subreddit = post["subreddit"].as_str().map(str::to_string);
    let author = post["author"].as_str().unwrap_or("[unknown]");
    let mut markdown = format!("# {title}\n\n");
    if let Some(sub) = &subreddit {
        markdown.push_str(&format!("r/{sub} · "));
    }
    markdown.push_str(&format!("u/{author}\n\n"));
    render_body(post, &mut walk, &mut markdown);

    if !comments.is_empty() {
        markdown.push_str("## Comments\n\n");
        for comment in comments {
            render_comment(comment, 0, &mut walk, &mut markdown);
        }
    }

    Thread {
        title,
        markdown,
        subreddit,
        published: post["created_utc"].as_u64().map(iso_time),
        captured_at: walk.newest_capture.map(iso_time),
        deleted_later: walk.deleted_later,
    }
}

/// The post's text. A crosspost has an empty body of its own, so the text
/// comes from the parent record; a link post shows its link.
fn render_body(post: &Value, walk: &mut Walk, out: &mut String) {
    let own = post["selftext"].as_str().unwrap_or_default();
    if !own.trim().is_empty() {
        out.push_str(walk.text(post, own).trim_end());
        out.push_str("\n\n");
        return;
    }
    let parent = &post["crosspost_parent_list"][0];
    if let Some(text) = parent["selftext"].as_str().filter(|t| !t.trim().is_empty()) {
        if let Some(sub) = parent["subreddit"].as_str() {
            out.push_str(&format!("Crossposted from r/{sub}:\n\n"));
        }
        out.push_str(walk.text(parent, text).trim_end());
        out.push_str("\n\n");
        return;
    }
    let flagged = walk.text(post, "");
    if !flagged.is_empty() {
        out.push_str(flagged);
        out.push_str("\n\n");
    } else if post["is_self"] == false
        && let Some(link) = post["url"].as_str()
    {
        out.push_str(&format!("Link: {link}\n\n"));
    }
}

/// One comment and its replies as nested bullets. Stubs for comments the
/// archive collapsed become a note, so the gap is visible.
fn render_comment(node: &Value, depth: usize, walk: &mut Walk, out: &mut String) {
    let indent = "  ".repeat(depth);
    let data = &node["data"];
    if node["kind"] == "more" {
        let hidden = data["count"]
            .as_u64()
            .filter(|n| *n > 0)
            .or_else(|| data["children"].as_array().map(|c| c.len() as u64))
            .unwrap_or(0);
        out.push_str(&format!("{indent}- [{hidden} more replies not shown]\n"));
        return;
    }
    walk.capture(data);
    let author = data["author"].as_str().unwrap_or("[unknown]");
    out.push_str(&format!("{indent}- **u/{author}**\n"));
    let body = walk.text(data, data["body"].as_str().unwrap_or_default());
    for line in body.lines() {
        out.push_str(format!("{indent}  {line}").trim_end());
        out.push('\n');
    }
    if let Some(replies) = data["replies"]["data"]["children"].as_array() {
        for reply in replies {
            render_comment(reply, depth + 1, walk, out);
        }
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a Unix timestamp.
fn iso_time(ts: u64) -> String {
    let secs = ts % 86400;
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        dates::epoch_days_to_date(ts / 86400),
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn comment(author: &str, body: &str, replies: Value) -> Value {
        json!({"kind": "t1", "data": {
            "author": author, "body": body, "replies": replies,
            "retrieved_on": 1000, "_meta": {"retrieved_2nd_on": 2000},
        }})
    }

    fn listing(children: Vec<Value>) -> Value {
        json!({"kind": "Listing", "data": {"children": children}})
    }

    fn post() -> Value {
        json!({
            "title": "Three failures", "subreddit": "gpu", "author": "op",
            "selftext": "Bought a card.\n\nIt failed.", "created_utc": 1785000000,
            "retrieved_on": 1500, "is_self": true,
        })
    }

    #[test]
    fn renders_nested_markdown_in_the_kagi_shape() {
        let tree = vec![
            comment(
                "a",
                "First.\n\nSecond paragraph.",
                listing(vec![comment("b", "Reply.", json!(""))]),
            ),
            comment("c", "Top level.", json!("")),
        ];
        let t = render(&post(), &tree);
        assert_eq!(
            t.markdown,
            "# Three failures\n\nr/gpu · u/op\n\nBought a card.\n\nIt failed.\n\n\
             ## Comments\n\n\
             - **u/a**\n  First.\n\n  Second paragraph.\n  - **u/b**\n    Reply.\n\
             - **u/c**\n  Top level.\n"
        );
        assert_eq!(t.title, "Three failures");
        assert_eq!(t.subreddit.as_deref(), Some("gpu"));
        assert_eq!(t.published.as_deref(), Some("2026-07-25T17:20:00Z"));
    }

    #[test]
    fn capture_time_is_the_newest_of_post_and_comments() {
        let t = render(&post(), &[comment("a", "x", json!(""))]);
        assert_eq!(t.captured_at, Some(iso_time(2000)));
        assert_eq!(render(&post(), &[]).captured_at, Some(iso_time(1500)));
    }

    #[test]
    fn crosspost_body_comes_from_the_parent() {
        let mut p = post();
        p["selftext"] = json!("");
        p["crosspost_parent_list"] =
            json!([{"selftext": "The real text.", "subreddit": "LocalLLM"}]);
        let t = render(&p, &[]);
        assert!(t
            .markdown
            .contains("Crossposted from r/LocalLLM:\n\nThe real text."));
    }

    #[test]
    fn link_post_shows_its_link() {
        let mut p = post();
        p["selftext"] = json!("");
        p["is_self"] = json!(false);
        p["url"] = json!("https://example.com/a");
        assert!(render(&p, &[])
            .markdown
            .contains("Link: https://example.com/a"));
    }

    #[test]
    fn deleted_text_is_withheld_where_it_stood() {
        let mut gone = comment("a", "My later regret.", json!(""));
        gone["data"]["_meta"]["was_deleted_later"] = json!(true);
        let mut p = post();
        p["_meta"] = json!({"was_deleted_later": true});
        let t = render(&p, &[gone, comment("b", "Still here.", json!(""))]);
        assert!(!t.markdown.contains("My later regret"));
        assert!(!t.markdown.contains("Bought a card"));
        assert!(t.markdown.contains("- **u/a**\n  [deleted later]\n"));
        assert!(t.markdown.contains("r/gpu · u/op\n\n[deleted later]\n\n"));
        assert!(t.markdown.contains("Still here."));
        assert_eq!(t.deleted_later, 2);
    }

    #[test]
    fn deleted_post_with_an_empty_body_is_still_counted() {
        let mut p = post();
        p["selftext"] = json!("");
        p["_meta"] = json!({"was_deleted_later": true});
        let t = render(&p, &[]);
        assert!(t.markdown.contains("[deleted later]"));
        assert_eq!(t.deleted_later, 1);
    }

    #[test]
    fn collapsed_comments_leave_a_note() {
        let more = json!({"kind": "more", "data": {"count": 0, "children": ["a", "b"]}});
        let t = render(&post(), &[comment("a", "x", listing(vec![more]))]);
        assert!(t.markdown.contains("  - [2 more replies not shown]\n"));
    }

    #[test]
    fn rate_limit_waits_once_and_only_within_the_budget() {
        let left = Duration::from_secs(30);
        assert_eq!(
            rate_limit_wait(Some("24"), left, false),
            Some(Duration::from_secs(24))
        );
        assert_eq!(rate_limit_wait(Some("24"), left, true), None);
        assert_eq!(rate_limit_wait(Some("29"), left, false), None);
        assert_eq!(rate_limit_wait(Some("soon"), left, false), None);
        assert_eq!(rate_limit_wait(None, left, false), None);
    }

    #[test]
    fn deep_trees_fail_to_parse_rather_than_overflow() {
        let mut node = comment("a", "x", json!(""));
        for _ in 0..40 {
            node = comment("a", "x", listing(vec![node]));
        }
        let text = json!({"data": [node]}).to_string();
        let err = serde_json::from_str::<Value>(&text).unwrap_err();
        assert!(err.to_string().contains("recursion limit"), "{err}");
    }
}
