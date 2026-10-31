//! Sites with no sanctioned anonymous route: content sits behind a login and
//! no public API serves it. Running the scraper chain against them only
//! collects login walls (or invites circumvention), so say so up front.
//! `-p <provider>` still forces the chain for a caller who wants to try.
//! The list is empirical: a site joins only when readers and scrapers came
//! back with login or block pages (checked 2026-10-31). X, Threads and
//! Facebook pages still render public content through Jina, so they stay on
//! the chain.

use super::{host_is, Route};
use crate::errors::SearchError;
use crate::types::SearchResult;
use async_trait::async_trait;
use url::Url;

const NAME: &str = "walled";

/// Domain and what the agent can do instead.
const WALLED: &[(&str, &str)] = &[
    (
        "linkedin.com",
        "LinkedIn serves profiles and posts only to logged-in members and has no public read API; for people search use `-m people`",
    ),
    (
        "quora.com",
        "Quora requires login and has no public API; search for the question text to find mirrors or citing pages",
    ),
    (
        "glassdoor.com",
        "Glassdoor reviews require login and it has no public API",
    ),
    (
        "instagram.com",
        "Instagram requires login; Meta's Graph API needs an approved app",
    ),
    (
        "discord.com",
        "Discord content is visible only to server members; there is no public web view",
    ),
    (
        "discord.gg",
        "Discord content is visible only to server members; there is no public web view",
    ),
];

pub struct Walled;

fn hint(url: &Url) -> Option<&'static str> {
    WALLED
        .iter()
        .find(|(domain, _)| host_is(url, domain))
        .map(|(_, hint)| *hint)
}

#[async_trait]
impl Route for Walled {
    fn name(&self) -> &'static str {
        NAME
    }

    fn matches(&self, url: &Url) -> bool {
        hint(url).is_some()
    }

    async fn fetch(&self, url: &Url) -> Result<Vec<SearchResult>, SearchError> {
        Err(SearchError::Blocked {
            provider: NAME,
            code: "no_sanctioned_route",
            message: format!(
                "no sanctioned route for {}: {}",
                url.host_str().unwrap_or_default(),
                hint(url).unwrap_or("the site requires login")
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn walled_sites_fail_with_a_hint() {
        let url = Url::parse("https://www.linkedin.com/in/someone").unwrap();
        assert!(Walled.matches(&url));
        let err = Walled.fetch(&url).await.unwrap_err();
        assert_eq!(err.error_code(), "no_sanctioned_route");
        assert!(err.to_string().contains("-m people"));
        assert!(!Walled.matches(&Url::parse("https://x.com/rustlang").unwrap()));
    }
}
