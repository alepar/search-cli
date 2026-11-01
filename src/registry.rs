//! Single source of truth for the mode map: what each mode does, what input
//! it takes, and which providers it routes to. The engine, `agent-info`, and
//! the filter-honesty warnings all read this table; SKILL.md and README
//! summarize it. A unit test pins the mode list so a new `Mode` variant
//! cannot ship without a registry entry.

use crate::types::Mode;

/// What the `-q` argument means for a mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// Free-text search query.
    Query,
    /// A full http(s) URL.
    Url,
}

impl InputKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::Url => "url",
        }
    }
}

/// How a mode combines its providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeKind {
    /// All providers queried in parallel, results deduped and ranked with
    /// reciprocal-rank fusion (cross-provider consensus ranks higher).
    Fused,
    /// Providers tried in listed order until one returns content.
    Chain,
    /// Exactly one provider serves the mode.
    Single,
}

impl MergeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fused => "fused",
            Self::Chain => "fallback_chain",
            Self::Single => "single_provider",
        }
    }
}

pub struct ModeSpec {
    pub mode: Mode,
    pub input: InputKind,
    pub merge: MergeKind,
    /// Providers the engine uses for this mode, in fan-out (Fused) or
    /// fallback (Chain) order.
    pub providers: &'static [&'static str],
    pub description: &'static str,
    pub when_to_use: &'static str,
}

pub const MODES: &[ModeSpec] = &[
    ModeSpec {
        mode: Mode::General,
        input: InputKind::Query,
        merge: MergeKind::Fused,
        providers: &[
            "parallel",
            "brave",
            "serper",
            "exa",
            "jina",
            "linkup",
            "kagi",
            "tavily",
            "perplexity",
        ],
        description: "Broad web search fused across all general-capable providers",
        when_to_use: "Default for any web lookup that doesn't fit a specialist mode.",
    },
    ModeSpec {
        mode: Mode::News,
        input: InputKind::Query,
        merge: MergeKind::Fused,
        providers: &[
            "parallel",
            "brave",
            "serper",
            "linkup",
            "kagi",
            "tavily",
            "perplexity",
        ],
        description: "News-specific endpoints of the news-capable providers",
        when_to_use: "Current events and anything where recency dominates; combine with -f day or -f week.",
    },
    ModeSpec {
        mode: Mode::Academic,
        input: InputKind::Query,
        merge: MergeKind::Fused,
        providers: &["exa", "serper", "tavily", "perplexity"],
        description: "Papers, preprints, and studies on the open web (semantic + web search)",
        when_to_use: "Finding research by topic. For Google Scholar records (citations, versions, PDFs) use scholar instead.",
    },
    ModeSpec {
        mode: Mode::People,
        input: InputKind::Query,
        merge: MergeKind::Single,
        providers: &["exa"],
        description: "Person and LinkedIn-profile search (Exa category search)",
        when_to_use: "Finding a specific person, their role, or their LinkedIn profile.",
    },
    ModeSpec {
        mode: Mode::Deep,
        input: InputKind::Query,
        merge: MergeKind::Fused,
        providers: &[
            "parallel",
            "brave",
            "serper",
            "exa",
            "linkup",
            "kagi",
            "tavily",
            "perplexity",
            "xai",
        ],
        description: "Maximum-coverage fan-out (web + X/Twitter + Brave grounding); waits for every provider, never cancels early",
        when_to_use: "Research where recall matters more than latency. Raise -c (e.g. -c 30) to keep more of the pool.",
    },
    ModeSpec {
        mode: Mode::Extract,
        input: InputKind::Url,
        merge: MergeKind::Chain,
        providers: &["stealth", "jina", "firecrawl", "browserless"],
        description: "Full page content as markdown. Sites with a sanctioned route are read through it first (Stack Exchange API, GitHub API, HN Algolia, Medium RSS, Telegram t.me/s, YouTube oEmbed + yt-dlp transcript; papers by DOI/arXiv/PubMed/PMC/PII via OpenAlex, Crossref, Unpaywall, Europe PMC and arXiv HTML); login-only sites (LinkedIn, Quora, Glassdoor, Instagram, Discord) fail fast with no_sanctioned_route. Otherwise tries stealth (opt-in build), Jina Reader, Firecrawl, and Browserless until one returns content; login walls, bot walls, empty JS shells and model-written text (code model_text: a reader or index gave a paraphrase, apology or 'unable to access' note instead of the page) count as failures (category blocked)",
        when_to_use: "Reading one specific page. -q must be a full URL. A blocked/no_sanctioned_route failure means the site walls off automated reading: use search results, an archive copy, or ask the user.",
    },
    ModeSpec {
        mode: Mode::Similar,
        input: InputKind::Url,
        merge: MergeKind::Single,
        providers: &["exa"],
        description: "Pages semantically similar to the given URL (Exa findSimilar)",
        when_to_use: "\"More like this page\": competitors, alternatives, related coverage. -q must be a full URL.",
    },
    ModeSpec {
        mode: Mode::Scrape,
        input: InputKind::Url,
        merge: MergeKind::Chain,
        providers: &["stealth", "jina", "firecrawl", "browserless"],
        description: "Alias of extract — identical provider chain and behavior",
        when_to_use: "Same as extract; the two names are interchangeable.",
    },
    ModeSpec {
        mode: Mode::Scholar,
        input: InputKind::Query,
        merge: MergeKind::Fused,
        providers: &["serper", "serpapi"],
        description: "Google Scholar records (citations, versions, PDF links)",
        when_to_use: "Scholarly metadata for a known paper/author. For topic discovery use academic.",
    },
    ModeSpec {
        mode: Mode::Patents,
        input: InputKind::Query,
        merge: MergeKind::Single,
        providers: &["serper"],
        description: "Google Patents search",
        when_to_use: "Prior art, patent families, inventor/assignee lookups.",
    },
    ModeSpec {
        mode: Mode::Images,
        input: InputKind::Query,
        merge: MergeKind::Single,
        providers: &["serper"],
        description: "Google Images search (returns image_url per result)",
        when_to_use: "Finding images; check the image_url field of each result.",
    },
    ModeSpec {
        mode: Mode::Places,
        input: InputKind::Query,
        merge: MergeKind::Single,
        providers: &["serper"],
        description: "Local businesses and places (Google Maps data)",
        when_to_use: "Businesses, opening hours, addresses near a location.",
    },
    ModeSpec {
        mode: Mode::Social,
        input: InputKind::Query,
        merge: MergeKind::Single,
        providers: &["xai"],
        description: "Live X/Twitter search via xAI Grok — an X summary plus cited posts",
        when_to_use: "What's being said on X, trending topics, account activity. Here -d/--exclude-domain filter X handles, not domains.",
    },
    ModeSpec {
        mode: Mode::Reddit,
        input: InputKind::Query,
        merge: MergeKind::Fused,
        providers: &["perplexity", "serper", "kagi", "tavily", "brave"],
        description: "Reddit threads, fused across search indexes scoped to reddit.com; one canonical URL per thread. Perplexity results carry indexed post and comment text. Never requests reddit.com",
        when_to_use: "Community opinion, experience reports, troubleshooting threads. Read a thread with -m extract on its URL (an indexed copy; Reddit blocks direct fetches). -d/--exclude-domain do not apply.",
    },
];

pub fn spec(mode: Mode) -> &'static ModeSpec {
    // The test below guarantees every Mode variant has an entry.
    MODES
        .iter()
        .find(|s| s.mode == mode)
        .expect("registry entry for every mode")
}

/// Which of the CLI's `-f`/`-d` filters a provider actually forwards to its
/// API. Providers not listed here (parallel, firecrawl, stealth, browserless)
/// apply none — the engine warns when their results enter a filtered search.
pub struct FilterSupport {
    pub freshness: bool,
    pub domains: bool,
    /// Caveat surfaced verbatim in warnings, e.g. xai remapping domains to handles.
    pub note: Option<&'static str>,
}

pub fn filter_support(provider: &str) -> FilterSupport {
    // Base name so "brave_llm_context" inherits brave's support.
    let base = provider.split("_llm_").next().unwrap_or(provider);
    match base {
        "brave" | "serper" | "serpapi" | "tavily" | "exa" | "perplexity" | "parallel"
        | "linkup" | "kagi" | "perplexity_search" => FilterSupport {
            freshness: true,
            domains: true,
            note: None,
        },
        "jina" => FilterSupport {
            freshness: false,
            domains: true,
            note: None,
        },
        "xai" => FilterSupport {
            freshness: true,
            domains: true,
            note: Some(
                "xai maps --domain/--exclude-domain to X handles, not web domains \
                 (max 20; --domain wins if both are given)",
            ),
        },
        _ => FilterSupport {
            freshness: false,
            domains: false,
            note: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::ValueEnum;

    #[test]
    fn every_mode_has_a_registry_entry() {
        for mode in Mode::value_variants() {
            assert!(
                MODES.iter().any(|s| s.mode == *mode),
                "mode {mode} missing from registry"
            );
        }
        assert_eq!(MODES.len(), Mode::value_variants().len());
    }

    #[test]
    fn registry_providers_exist() {
        // Every provider named in the registry must be a real provider name.
        let known = [
            "parallel",
            "brave",
            "serper",
            "exa",
            "jina",
            "linkup",
            "kagi",
            "firecrawl",
            "tavily",
            "serpapi",
            "perplexity",
            "browserless",
            "stealth",
            "xai",
            "reddit",
        ];
        for spec in MODES {
            for p in spec.providers {
                assert!(known.contains(p), "unknown provider {p} in registry");
            }
        }
    }

    #[test]
    fn scrape_is_exact_alias_of_extract() {
        assert_eq!(spec(Mode::Scrape).providers, spec(Mode::Extract).providers);
    }

    #[test]
    fn url_modes_are_marked() {
        for m in [Mode::Extract, Mode::Scrape, Mode::Similar] {
            assert_eq!(spec(m).input, InputKind::Url);
        }
    }
}
