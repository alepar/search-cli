//! Recognise pages that are not content: bot-protection interstitials, login
//! walls and empty JS shells. Scrapers often return these with HTTP 200, so the
//! extract chain must look at what came back, not just the status.

use crate::types::{FailureCategory, ProviderFailure};

/// Why a fetched page is not the content that was asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wall {
    /// Anti-bot challenge or block page (Cloudflare, DataDome, PerimeterX,
    /// Akamai, AWS WAF, a site's own block page).
    Bot,
    /// The site wants a logged-in session.
    Login,
    /// Next to no text: a client-rendered app shell.
    JsShell,
    /// Binary or undecodable bytes.
    Garbage,
    /// A language model's note about the page (a paraphrase, an apology, an
    /// "unable to access" message), not the page.
    ModelText,
}

impl Wall {
    pub fn code(self) -> &'static str {
        match self {
            Self::Bot => "bot_wall",
            Self::Login => "login_wall",
            Self::JsShell => "js_shell",
            Self::Garbage => "garbage_content",
            Self::ModelText => "model_text",
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Self::Bot => "an anti-bot challenge or block page instead of content",
            Self::Login => "a login wall instead of content",
            Self::JsShell => "an empty JavaScript app shell instead of content",
            Self::Garbage => "binary or undecodable content",
            Self::ModelText => "model-written text instead of the page",
        }
    }

    fn category(self) -> FailureCategory {
        match self {
            Self::Bot | Self::Login | Self::ModelText => FailureCategory::Blocked,
            Self::JsShell | Self::Garbage => FailureCategory::Parse,
        }
    }

    pub fn failure(self, provider: &str, http_status: Option<u16>) -> ProviderFailure {
        ProviderFailure {
            provider: provider.to_string(),
            category: self.category(),
            http_status,
            code: self.code().to_string(),
            reason: format!("{provider} returned {}", self.describe()),
            retryable: false,
        }
    }
}

/// Interstitial and block-page phrases. Only trusted on short pages: a real
/// article that mentions Cloudflare is long.
const BOT_MARKERS: &[&str] = &[
    // Cloudflare
    "checking your browser before accessing",
    "just a moment...",
    "verifying you are human",
    "enable javascript and cookies to continue",
    "cf-browser-verification",
    "attention required! | cloudflare",
    // DataDome
    "captcha-delivery.com",
    // HUMAN / PerimeterX
    "px-captcha",
    "press & hold",
    "are you a robot",
    // Akamai and generic edge blocks
    "access denied",
    "you don't have permission to access",
    "pardon our interruption",
    // AWS WAF, Imperva and other JS challenges
    "client challenge",
    "request unsuccessful. incapsula",
    "please verify you are a human",
    // Site-specific block pages (Reddit, Quora, Glassdoor, Google)
    "blocked by network security",
    "you've been blocked",
    "you have been blocked",
    "blocked in error",
    "mistakenly blocked",
    "detected unusual traffic",
];

/// A page this short is not an extraction, whatever it says.
const SHELL_MAX_CHARS: usize = 80;
/// Challenge pages are short; above this, markers are probably prose.
const BOT_MAX_CHARS: usize = 4000;
/// Login walls carry some chrome (footer links, a profile name), but not much.
const LOGIN_MAX_CHARS: usize = 2500;

/// Openings that only a language model writes about a page it was handed.
const MODEL_OPENERS: &[&str] = &[
    "here's a rewritten version",
    "here is a rewritten version",
    "based on the provided content",
    "based on the provided text",
    "as an ai,",
    "as an ai language model",
    "as an ai model",
    "as an ai assistant",
    "i can't assist with",
    "i cannot assist with",
];

/// Openings that people write too ("I can't access my router"), so they count
/// only alongside a [`MODEL_PIVOTS`] phrase.
const MODEL_REFUSALS: &[&str] = &[
    "i'm unable to access",
    "i am unable to access",
    "i was unable to access",
    "i'm not able to access",
    "i am not able to access",
    "i cannot access",
    "i can't access",
    "i could not access",
    "i couldn't access",
    "i don't have access to",
    "i do not have access to",
    "i'm unable to browse",
    "i am unable to browse",
    "i cannot browse",
    "i can't browse",
];

/// What a model goes on to say after refusing.
const MODEL_PIVOTS: &[&str] = &[
    "here's a rewritten",
    "here is a rewritten",
    "here's a summary",
    "here is a summary",
    "rewritten version",
    "based on the provided",
    "as an ai",
    "paste the text",
    "paste the content",
];

/// Apologies a model puts in front of a refusal.
const MODEL_APOLOGIES: &[&str] = &[
    "i'm sorry, but ",
    "i am sorry, but ",
    "sorry, but ",
    "i apologize, but ",
    "unfortunately, ",
];

/// Characters of body text examined. A model's note about the page comes
/// first; further down the same words are a quotation or a discussion.
const OPENING_CHARS: usize = 300;

/// True when text from a third-party reader or search index opens in a model's
/// voice. Only the start of the body counts, so a page that quotes or discusses
/// a refusal passes. A model's rewrite that leaves no such note is not caught.
pub fn is_model_text(content: &str) -> bool {
    let opening = body_opening(content);
    let body = MODEL_APOLOGIES
        .iter()
        .find_map(|a| opening.strip_prefix(a))
        .unwrap_or(&opening);
    MODEL_OPENERS.iter().any(|o| body.starts_with(o))
        || (MODEL_REFUSALS.iter().any(|r| body.starts_with(r))
            && MODEL_PIVOTS.iter().any(|p| body.contains(p)))
}

/// The start of the body, lowercased, with straight apostrophes.
fn body_opening(content: &str) -> String {
    content
        .lines()
        .map(str::trim)
        .skip_while(|line| is_preamble(line))
        .flat_map(|line| line.chars().chain(['\n']))
        .take(OPENING_CHARS)
        .flat_map(char::to_lowercase)
        .map(|c| if c == '\u{2019}' { '\'' } else { c })
        .collect()
}

/// Lines ahead of the body: the title and the index's metadata. Its comments
/// heading is not one, because what follows it is a commenter's words.
fn is_preamble(line: &str) -> bool {
    let bold = line.len() > 4 && line.starts_with("**") && line.ends_with("**");
    line.is_empty()
        || matches!(line, "..." | "---")
        || (line.starts_with('#') && !line.eq_ignore_ascii_case("### comments"))
        || bold
        || line.starts_with("author: ")
}

/// Classify text from a third-party reader or scraper: [`classify_content`]
/// plus model-written text. First-party APIs use [`classify_content`] alone.
pub fn classify_reader_text(content: &str) -> Option<Wall> {
    if is_model_text(content) {
        return Some(Wall::ModelText);
    }
    classify_content(content)
}

/// Classify extracted page text. `None` means it looks like content.
pub fn classify_content(content: &str) -> Option<Wall> {
    let sample: Vec<char> = content.chars().take(BOT_MAX_CHARS).collect();
    if sample.len() > 200 {
        let garbage = sample
            .iter()
            .filter(|c| **c == '\u{FFFD}' || (c.is_control() && !c.is_whitespace()))
            .count();
        if garbage * 20 > sample.len() {
            return Some(Wall::Garbage);
        }
    }

    // Reader output is markdown: link targets are not visible text, and on
    // a login page they can be most of the bytes.
    let visible = strip_link_targets(content);
    let len = visible.trim().chars().count();
    if len < SHELL_MAX_CHARS {
        return Some(Wall::JsShell);
    }
    if len >= BOT_MAX_CHARS {
        return None;
    }
    let lower = visible.to_lowercase();
    if BOT_MARKERS.iter().any(|m| lower.contains(m)) {
        return Some(Wall::Bot);
    }
    if len < LOGIN_MAX_CHARS && is_login_wall(&lower) {
        return Some(Wall::Login);
    }
    None
}

/// Drop markdown link and image targets: "[text](https://...)" -> "[text]".
fn strip_link_targets(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut rest = md;
    while let Some(i) = rest.find("](") {
        out.push_str(&rest[..=i]);
        let after = &rest[i + 2..];
        match after.find(')') {
            Some(close) => rest = &after[close + 1..],
            None => {
                rest = after;
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

fn is_login_wall(lower: &str) -> bool {
    const STRONG: &[&str] = &[
        "sign in to continue",
        "log in to continue",
        "you must log in",
        "join now to see",
        "authwall",
        "log in or sign up",
    ];
    if STRONG.iter().any(|m| lower.contains(m)) {
        return true;
    }
    // "Log in" plus "Sign up" chrome with nothing much else around it.
    (lower.contains("log in") || lower.contains("sign in"))
        && [
            "sign up",
            "create account",
            "create new account",
            "forgot password",
        ]
        .iter()
        .any(|m| lower.contains(m))
}

/// Classify an HTTP response by status and headers, before reading the body.
/// `header` returns a header's value by lowercase name (for `set-cookie`, any
/// one of them joined is fine).
pub fn classify_response(status: u16, header: impl Fn(&str) -> Option<String>) -> Option<Wall> {
    let has = |name: &str, needle: &str| {
        header(name).is_some_and(|v| v.to_ascii_lowercase().contains(needle))
    };
    if has("cf-mitigated", "challenge") || has("x-amzn-waf-action", "") {
        return Some(Wall::Bot);
    }
    if status == 999 {
        // LinkedIn's non-standard "go away" status.
        return Some(Wall::Login);
    }
    if !matches!(status, 401 | 403 | 405 | 429 | 503) {
        return None;
    }
    if header("x-datadome").is_some()
        || has("server", "datadome")
        || has("set-cookie", "_px")
        || has("server", "akamaighost")
        || has("server", "cloudflare")
        || has("x-iinfo", "")
    {
        return Some(Wall::Bot);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prose(n: usize) -> String {
        "Real sentence with ordinary words. ".repeat(n)
    }

    #[test]
    fn binary_and_cloudflare_pages_are_walls() {
        let binary: String = "\u{FFFD}x\u{FFFD}\u{FFFD}".repeat(100);
        assert_eq!(classify_content(&binary), Some(Wall::Garbage));
        let cf = format!(
            "## Checking your browser before accessing example.com\n{}",
            prose(3)
        );
        assert_eq!(classify_content(&cf), Some(Wall::Bot));
    }

    #[test]
    fn other_vendors_block_pages_are_bot_walls() {
        for page in [
            "Please enable JS and disable any ad blocker. geo.captcha-delivery.com/captcha",
            "Are you a robot? Please make sure your browser supports JavaScript. px-captcha",
            "Access Denied. You don't have permission to access this page. Reference #18.abc",
            "Client Challenge. A required part of this site couldn't load.",
            "whoa there, pardner! You've been blocked by network security.",
        ] {
            let padded = format!("{page} {}", "x ".repeat(40));
            assert_eq!(classify_content(&padded), Some(Wall::Bot), "{page}");
        }
    }

    #[test]
    fn login_walls_are_detected() {
        let instagram = format!(
            "NASA (@nasa) • Instagram photos and videos Log In Sign Up {}",
            "Meta About Blog Jobs Help API Privacy Terms ".repeat(10)
        );
        assert_eq!(classify_content(&instagram), Some(Wall::Login));
        let linkedin = format!("Join now to see what you are missing. {}", prose(3));
        assert_eq!(classify_content(&linkedin), Some(Wall::Login));
        // Instagram via a reader: a login form plus footer links.
        let form = format!(
            "Log into Instagram\n\nMobile number, username or email\n\nPassword\n\nLog in\n\n\
             [Forgot password?]\n\n[Create new account]\n\n{}",
            "[About] [Blog] [Jobs] [Help] ".repeat(10)
        );
        assert_eq!(classify_content(&form), Some(Wall::Login));
    }

    #[test]
    fn link_targets_do_not_count_as_text() {
        let links = "[Privacy](https://example.com/a-very-long-tracking-url?x=1) ".repeat(80);
        let page =
            format!("Agree & Join LinkedIn. Sign in. [Forgot password?](https://x/y) {links}");
        assert!(page.len() > 4000);
        assert_eq!(classify_content(&page), Some(Wall::Login));
        assert_eq!(strip_link_targets("a [b](c) d ![e](f)"), "a [b] d ![e]");
    }

    #[test]
    fn shells_are_detected() {
        assert_eq!(classify_content("reddit"), Some(Wall::JsShell));
        assert_eq!(classify_content("   \n  "), Some(Wall::JsShell));
    }

    #[test]
    fn real_content_passes() {
        let article = format!("How Cloudflare works. Access denied errors. {}", prose(300));
        assert_eq!(classify_content(&article), None);
        let short = "A short but real paragraph about Rust ownership, borrowing and lifetimes, \
                     which is all this page says.";
        assert_eq!(classify_content(short), None);
        // A long page with a "Log in" link in its nav is still content.
        let with_nav = format!("Log in Sign up {}", prose(120));
        assert_eq!(classify_content(&with_nav), None);
    }

    /// The first part of the copy that started this: index metadata, title,
    /// then the note.
    const REWRITE: &str = "**subreddit: /r/gpu**\n...\n### RTX \"pro\" 6000 WS.\nI'm unable to \
                           access the original post from r/AIProgrammingHardware, but here's a \
                           rewritten version based on the provided content:\n---\n**A Journey**\n\
                           In 2025, I purchased a brand-new RTX Pro 6000 WS.";

    #[test]
    fn model_notes_at_the_opening_are_model_text() {
        assert!(is_model_text(REWRITE));
        for page in [
            "Here's a rewritten version of the post:\n\nI bought a card.",
            "# Title\n\nBased on the provided content, the author bought a card.",
            "author: someone\n### Title\nAs an AI language model, I cannot read this page.",
            "As an AI, I can't open links.",
            "I'm sorry, but I can't assist with that.",
            "I apologize, but I cannot access that URL. Please paste the text here.",
            "I can\u{2019}t browse the web, but here\u{2019}s a summary of the likely content.",
            "I cannot access external sites as an AI. Unfortunately that is all.",
            "I don't have access to the article. Based on the provided title, it covers GPUs.",
            "Unfortunately, I can't access the page. Here is a summary from the snippet.",
        ] {
            assert!(is_model_text(page), "{page}");
        }
    }

    #[test]
    fn people_saying_similar_things_is_not_model_text() {
        for page in [
            // A real comment on a Lemmy thread.
            "I am unable to access the original post.\n\nIf I remember right, there was no \
             source code for this version of the extension.",
            "I can't access my router's admin page since the update, here's a photo of the LED.",
            "I cannot access the BIOS. Sorry for the long post.",
            "I'm sorry, but I can't access the site from work, so no screenshots.",
            "As an AI researcher, I spend my days reading papers.",
            "As an AI engineer I would not ship this.",
            "Here's a rewrite of the function using iterators.",
        ] {
            assert!(!is_model_text(page), "{page}");
        }
    }

    #[test]
    fn model_notes_past_the_opening_are_not_model_text() {
        // An article about refusals.
        let article = format!(
            "# Why chatbots refuse\n\nModels often answer \"I'm unable to access that page, but \
             here's a rewritten version\" when they cannot fetch a URL. {}",
            prose(40)
        );
        assert!(!is_model_text(&article));
        // A quotation at the top of a post, and a model's words further down.
        assert!(!is_model_text(
            "> As an AI language model, I cannot.\n\nLook at this."
        ));
        let late = format!("{}\nAs an AI language model, I cannot.", prose(10));
        assert!(!is_model_text(&late));
        // A commenter quoting one, with the post itself elided by the index.
        let thread = "### Odd chatbot\n...\n### Comments\nAs an AI language model, I cannot.\n";
        assert!(!is_model_text(thread));
    }

    #[test]
    fn reader_text_reports_model_text_at_any_length() {
        let long = format!("{REWRITE}\n{}", prose(300));
        assert!(long.chars().count() > BOT_MAX_CHARS);
        assert_eq!(classify_reader_text(&long), Some(Wall::ModelText));
        assert_eq!(classify_reader_text("reddit"), Some(Wall::JsShell));
        assert_eq!(classify_reader_text(&prose(10)), None);
        // First-party text keeps the plain classifier.
        assert_eq!(classify_content(&long), None);
        let failure = Wall::ModelText.failure("jina", None);
        assert_eq!(failure.code, "model_text");
        assert_eq!(failure.category, FailureCategory::Blocked);
    }

    #[test]
    fn response_headers_identify_walls() {
        let none = |_: &str| None;
        assert_eq!(classify_response(200, none), None);
        assert_eq!(classify_response(404, none), None);
        assert_eq!(classify_response(403, none), None);
        assert_eq!(classify_response(999, none), Some(Wall::Login));

        let cf = |n: &str| (n == "cf-mitigated").then(|| "challenge".to_string());
        assert_eq!(classify_response(403, cf), Some(Wall::Bot));
        let waf = |n: &str| (n == "x-amzn-waf-action").then(|| "challenge".to_string());
        assert_eq!(classify_response(202, waf), Some(Wall::Bot));
        let dd = |n: &str| (n == "x-datadome").then(|| "protected".to_string());
        assert_eq!(classify_response(403, dd), Some(Wall::Bot));
        assert_eq!(classify_response(200, dd), None);
        let px = |n: &str| (n == "set-cookie").then(|| "_pxhd=abc; path=/".to_string());
        assert_eq!(classify_response(403, px), Some(Wall::Bot));
    }
}
