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
}

impl Wall {
    pub fn code(self) -> &'static str {
        match self {
            Self::Bot => "bot_wall",
            Self::Login => "login_wall",
            Self::JsShell => "js_shell",
            Self::Garbage => "garbage_content",
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Self::Bot => "an anti-bot challenge or block page instead of content",
            Self::Login => "a login wall instead of content",
            Self::JsShell => "an empty JavaScript app shell instead of content",
            Self::Garbage => "binary or undecodable content",
        }
    }

    fn category(self) -> FailureCategory {
        match self {
            Self::Bot | Self::Login => FailureCategory::Blocked,
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
    // Site-specific block pages
    "blocked by network security",
    "you've been blocked",
];

/// A page this short is not an extraction, whatever it says.
const SHELL_MAX_CHARS: usize = 80;
/// Challenge pages are short; above this, markers are probably prose.
const BOT_MAX_CHARS: usize = 4000;
/// Login walls carry some chrome (footer links, a profile name), but not much.
const LOGIN_MAX_CHARS: usize = 2500;

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

    let len = content.trim().chars().count();
    if len < SHELL_MAX_CHARS {
        return Some(Wall::JsShell);
    }
    if len >= BOT_MAX_CHARS {
        return None;
    }
    let lower = content.to_lowercase();
    if BOT_MARKERS.iter().any(|m| lower.contains(m)) {
        return Some(Wall::Bot);
    }
    if len < LOGIN_MAX_CHARS && is_login_wall(&lower) {
        return Some(Wall::Login);
    }
    None
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
}
