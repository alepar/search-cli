# Empirical probe of retrieval resistance (2026-10-31)

Source: first-hand test run from the developer's workstation (residential/home ISP connection, Fedora Linux), 2026-10-31. Not a third-party source; cite as "our probe, 2026-10-31".

## Method and limits

- The stock search-cli extract chain could not be exercised end to end on this host: no provider API keys are configured (Jina, Firecrawl, Browserless all `configured: false` in `search doctor`), and the `stealth` feature (wreq Chrome136 TLS impersonation) needs BoringSSL + a C/C++ toolchain that this immutable host lacks.
- A Chrome-TLS-impersonation probe (curl_cffi, equivalent to the stealth rung) and a keyless Jina Reader probe were prepared but NOT run: the agent's safety policy refused the impersonation run as bot-check circumvention. That refusal is itself a data point about the posture of the stealth rung.
- What was run: a plain, honestly identified client (User-Agent `curl/8.9.1`, no impersonation), one GET per URL, following redirects, from a residential IP. Verdict = HTTP status + visible-text length + presence of challenge markers (Cloudflare, DataDome `captcha-delivery.com`, PerimeterX `px-captcha`/"Press & Hold", Akamai "Access Denied", login walls) + an expected keyword.
- Single sample per site; results reflect this IP at this time, not a rate-limit study.

## Results: plain client against the page itself

| Site | HTTP | Outcome | Evidence |
|---|---|---|---|
| Wikipedia article (control) | 200 | full content | 31 KB text |
| reddit.com subreddit page | 200 | empty shell | 6 chars visible text (JS app) |
| old.reddit.com | 403 | blocked | "You've been blocked by network security" |
| reddit `.json` (top.json) | 403 | blocked | same Reddit block page |
| reddit `/r/rust/.rss` | 200 | works | 52 KB Atom feed |
| x.com profile | 200 | shell + login prompt | 2 KB, "Log in / Sign up"; no posts |
| LinkedIn company page | 999 | blocked | LinkedIn's non-standard 999 / authwall |
| Medium publication page | 403 | blocked | "Attention Required! | Cloudflare" |
| Medium RSS (`/feed/<pub>`) | 200 | works | 153 KB feed with full post bodies |
| Stack Overflow question | 403 | blocked | Cloudflare "Just a moment..." challenge |
| Stack Exchange API v2.3 (keyless) | 200 | works | question with body |
| Quora question | 403 | blocked | Cloudflare challenge |
| NYT article | 403 | blocked | DataDome (`captcha-delivery.com`) |
| WSJ home | 403 | blocked | (Dow Jones; vendor not identified in body) |
| Bloomberg home | 403 | blocked | PerimeterX ("Are you a robot", px-captcha) |
| FT home | 403 | blocked | |
| Glassdoor reviews | 403 | blocked | Cloudflare "Enable JavaScript and cookies" |
| Instagram profile | 200 | login shell | 731 chars, "Log In / Sign Up" only |
| Facebook page | 200 | empty shell | 52 chars |
| Threads profile | 200 | empty shell | 37 chars |
| YouTube watch page | 200 | title only | 252 chars visible; transcript not in HTML |
| YouTube oEmbed | 200 | works | title/author metadata |
| GitHub repo page | 200 | works | |
| GitHub REST API (unauthenticated) | 200 | works | repo JSON |
| ScienceDirect (Elsevier) article | 403 | blocked | |
| Springer article | 200 | works | 181 KB text |
| IEEE Xplore document | 202 | empty | 0 chars (JS challenge/async) |
| JSTOR stable page | 200 | challenge | "Client Challenge ... couldn't load" |
| ResearchGate publication | 403 | blocked | |
| arXiv abs page | 200 | works | |
| arXiv export API | 200 | works | Atom metadata |
| OpenAlex work by DOI | 200 | works | |
| Unpaywall by DOI | 200 | works | |
| Semantic Scholar Graph API (keyless) | 200 | works | |
| Europe PMC REST | 200 | works | |
| HN item page | 200 | works | |
| HN Algolia API | 200 | works | |
| Substack post (custom domain) | 200 | works | 13 KB text |
| G2 reviews | 403 | blocked | DataDome |
| Zillow listing | 403 | blocked | PerimeterX "Press & Hold" |
| Indeed search | 403 | blocked | "Access Denied" |
| Home Depot category | 403 | blocked | (Akamai-style) |
| Discord invite page | 200 | empty shell | 49 chars |
| Telegram `t.me/s/<channel>` | 200 | works | 16 KB public channel preview |
| Wayback availability API | 429 | rate-limited | 429 on the first request from this IP, twice; CDX API timed out |
| X oEmbed (publish.twitter.com) | 301 | inconclusive | redirect, not followed (test used a fake status id) |

## Observations

1. Of ~35 page URLs, a plain honest client got usable content from about 10 (Wikipedia, GitHub, Springer, arXiv, Substack, HN, Telegram previews, and the API/feeds). About 17 returned 403/999 bot-protection or block pages. The rest returned JS shells or login walls with 200 status.
2. Several 200 responses are login walls or empty JS shells (Instagram, Facebook, Threads, X, Discord, reddit.com, YouTube). The current `scrape_rejection` heuristic in `src/engine.rs` only matches Cloudflare challenge phrases and binary garbage. A short "Log In / Sign Up" shell would pass as a successful extraction and stop the chain from escalating. JSTOR's "Client Challenge", DataDome, PerimeterX ("Press & Hold", px-captcha), Akamai "Access Denied" and Reddit's "blocked by network security" are not in the marker list, though most of those also come with a non-2xx status that already fails the stealth rung.
3. Sanctioned routes work keyless where the HTML does not: Reddit RSS, Medium RSS, the Stack Exchange API, GitHub API, HN Algolia, arXiv/OpenAlex/Unpaywall/Semantic Scholar/Europe PMC, Telegram `t.me/s/` and YouTube oEmbed.
4. Reddit blocks even its own `.json` endpoint and old.reddit for an unauthenticated non-browser client from this IP, but still serves `.rss`.
