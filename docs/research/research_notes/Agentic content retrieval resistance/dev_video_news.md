# Agentic content retrieval resistance: developer/knowledge sites, video platforms, news publishers (as of 2026-10-31)

Method note: besides web sources, I ran live probes with `curl` on 2026-10-31 from the research host (a Linux workstation; the egress IP type was not verified, but it is not a known cloud range). Results marked **[live probe 2026-10-31]** are first-hand observations from one IP at one moment. They are not vendor statements and may differ from datacenter IPs. Where a page could not be fetched directly (api.stackexchange.com and developer.nytimes.com both refused the fetch tool), the figures come from secondary sources and are marked as such.

## Stack Overflow / Stack Exchange: what restrictions apply (2024-2026), and does the keyless API v2.3 still work?

### Takeaway
The HTML site is hostile to anything that is not a real browser. stackoverflow.com and superuser.com return a Cloudflare managed challenge (403, `cf-mitigated: challenge`) even to a browser User-Agent sent from curl. The robots.txt served to non-browsers says `Disallow: /` with `Content-signal: search=no, ai-train=no`. The public API v2.3, however, still works without a key at about 300 requests/day/IP (10,000 with a free app key). That API is the legitimate and practical route for a research CLI. Data dumps moved behind a login and an agreement in 2024.

### Cited Findings
- **[live probe 2026-10-31]** `GET https://api.stackexchange.com/2.3/questions?...&site=stackoverflow` with no key returned 200 and JSON. `/2.3/info` reported `"quota_max":300`, `"quota_remaining":297`. So the keyless quota is 300/day for this IP. — (probe of api.stackexchange.com)
- Keyless quota is 300 requests/day per IP, reset at UTC midnight. A free app key registered on stackapps.com raises it to 10,000/day per IP. Clients must honour the `backoff` field or risk HTTP 503 throttling and IP bans. The no-key quota "is meant to facilitate experimentation". — [api-evangelist rate-limit YAML (secondary, cites api.stackexchange.com/docs/throttle)](https://raw.githubusercontent.com/api-evangelist/stackexchange/refs/heads/main/rate-limits/stackexchange-rate-limits.yml); [apis.io](https://apis.io/rate-limits/stackexchange/stackexchange-rate-limits/). The official page could not be fetched here, but the live `quota_max:300` matches.
- **[live probe 2026-10-31]** `https://stackoverflow.com/questions/11227809` with a Chrome UA returned 403 with headers `server: cloudflare` and `cf-mitigated: challenge`. superuser.com also returned 403, while serverfault.com returned 200. Cloudflare challenges on the HTML sites are inconsistent across the network but are active on the biggest ones.
- **[live probe 2026-10-31]** `https://stackoverflow.com/robots.txt` fetched with curl returned **HTTP 418** with this body: `License: https://stackoverflow.com/license.xml` / `User-agent: *` / `Content-signal: search=no, ai-train=no` / `Disallow: /`. `license.xml` is an RSL (Really Simple Licensing, rslstandard.org) document that points at CC BY-SA 4.0 terms. A browser presumably gets a different robots.txt; this is the bot-facing variant. The site uses Cloudflare Content Signals plus RSL.
- Data dumps: the quarterly upload to the Internet Archive stopped after April 2024. Dumps moved to the user profile behind a login and a download agreement. Reporting from July 2024 says the agreement was narrowed from "non-commercial" to restrictions on LLM training. Critics say this conflicts with CC BY-SA. SE said: "We are asking politely that you not, but realistically speaking, there's nothing we can do to prevent you from sharing it". — [DevClass, 2024-07-30](https://devclass.com/2024/07/30/stack-exchange-restricts-access-to-dump-of-user-contributed-data-as-critics-complain-license-permits-reuse-for-any-purpose/); [Archiveteam wiki](https://wiki.archiveteam.org/index.php/Stack_Exchange) (says the 2024 change excludes commercial use, distribution and LLM training; this conflicts with DevClass on scope); [Feep blog, 2025-02-20](https://search.feep.dev/blog/post/2025-02-20-state-of-stackexchange)
- OverflowAPI is the paid, subscription product for continuous access to the public dataset for AI training and fine-tuning. The OpenAI deal (2024) used it. — [Medium (secondary)](https://fferoz.medium.com/stack-overflows-quiet-collapse-when-ai-eats-the-internet-that-trained-it-af715c2389d5). A claimed price of "$0.01-0.10/query" from a stats site is **unverified**: [skillademia](https://www.skillademia.com/statistics/stackoverflow-statistics/).
- Stack Overflow runs a beta MCP server with `so_search` (lexical search) and `get_content` (questions, answers and comments by ID), capped at **100 calls/day per Stack Exchange user**, with login via mcp-remote. Higher volume goes to sales. — [mcpservers.org listing (third-party, quoting SO text)](https://mcpservers.org/es/servers/api-stackexchange-com-docs-mcp-server)
- After the 2023 moderator strike, SE committed "to continue to provide data and API access". — [Wikipedia: Stack Overflow](https://en.wikipedia.org/wiki/Stack_Overflow)

### Inferences
- For a research CLI, the sanctioned route is: search with `/2.3/search/advanced` or `/2.3/similar`, then fetch bodies with `filter=withbody` on `/questions/{ids}` and `/questions/{ids}/answers`. Batch up to 100 IDs per call so 300/day stretches far. Ship or let users configure a stackapps key for 10k/day. Scraping HTML pages will mostly hit the Cloudflare challenge, and the bot-facing robots.txt explicitly disallows it.
- Content is CC BY-SA, so attribution (author and link) should be preserved in extracted output.

### Gaps
- I could not read the official api.stackexchange.com/docs/throttle or the current API Terms of Use. Whether the 2024-2026 API terms explicitly forbid using API output in AI products (beyond the OverflowAPI upsell) is unconfirmed.
- No official announcement dating when Cloudflare challenges started on stackoverflow.com was found.
- Launch date and official documentation for the SO MCP server were not found.

## GitHub: unauthenticated vs token rate limits (REST, GraphQL, raw, web), and code search auth

### Takeaway
Unauthenticated REST is 60 requests/hour per IP. A token gives 5,000/hour. Code search via the API **requires auth** (401 without). On 2025-05-08 GitHub tightened unauthenticated limits across REST, HTTPS clone and raw.githubusercontent.com, citing scraping. raw.githubusercontent.com does not accept auth headers, so the token-respecting route for file contents is the REST contents API. robots.txt does not ban AI crawlers; it gives GPTBot, ClaudeBot and others `Crawl-delay: 1`.

### Cited Findings
- REST primary limits: unauthenticated **60/hour per originating IP**. PAT/OAuth user token **5,000/hour** (15,000 for Enterprise Cloud-owned apps). GitHub App installation 5,000 scaling to 12,500. `GITHUB_TOKEN` in Actions 1,000/hour/repo. — [GitHub Docs: REST rate limits](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api)
- Secondary limits: at most 100 concurrent requests (REST and GraphQL combined). REST allows 900 points/min per endpoint (GET = 1 point). GraphQL allows 2,000 points/min. CPU time is capped at 90 s per 60 s. Search endpoints have separate, stricter limits (not quantified on that page). — [GitHub Docs](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api)
- **[live probe 2026-10-31]** `api.github.com/repos/torvalds/linux` unauthenticated returned `x-ratelimit-limit: 60`, `x-ratelimit-resource: core`. `api.github.com/search/code?q=...` without a token returned `{"message":"Requires authentication","status":"401"}`.
- 2025-05-08 changelog, "Updated rate limits for unauthenticated requests": the tighter limits apply to HTTPS cloning, anonymous REST calls and **downloads from raw.githubusercontent.com**, motivated by "an increase in scraping activity". — [GitHub Changelog 2025-05-08](https://github.blog/changelog/2025-05-08-updated-rate-limits-for-unauthenticated-requests/); [HN discussion](https://news.ycombinator.com/item?id=43936992)
- raw.githubusercontent.com does not use auth headers to lift limits. Community advice is to use the REST contents endpoint with a token instead. 429s from raw were reported in real projects in 2025. — [GitHub community discussion #160828](https://github.com/orgs/community/discussions/160828); [bazarr issue #3057](https://github.com/morpheus65535/bazarr/issues/3057); 429s triggered by an `Accept-Language: zh-CN` header: [discussion #157887](https://github.com/orgs/community/discussions/157887)
- **[live probe 2026-10-31]** A `HEAD` to `raw.githubusercontent.com/torvalds/linux/master/README` returned **429**, while a `GET` of the same URL and of a cpython file returned 200 (Fastly cache HIT). Raw limiting appears to depend on request shape and cache state; no rate-limit headers are exposed.
- **[live probe 2026-10-31]** `github.com/.../blob/...` HTML returned 200 to a browser UA. `github.com/robots.txt` puts GPTBot, OAI-SearchBot, ClaudeBot, anthropic-ai and PerplexityBot in a group with `Crawl-delay: 1` and specific Allow lines, with no blanket `Disallow: /` near the top of the file.

### Inferences
- For a CLI: accept a `GITHUB_TOKEN`/`gh auth token` and prefer REST/GraphQL with it (5,000/h). Use `/repos/{o}/{r}/contents/{path}` with `Accept: application/vnd.github.raw` for files. Treat raw.githubusercontent.com as a best-effort fallback for the no-token case. Code search is unavailable without a token, so the CLI must degrade gracefully or prompt.

### Gaps
- Exact current numeric limits for unauthenticated raw.githubusercontent.com and for web-page HTML are not published. The GitHub Docs REST page does not mention them.
- The current GraphQL primary point budget (historically 5,000 points/hour for users) was not confirmed on the fetched page.
- Search API numeric limits (historically 10/min unauthenticated and 30/min authenticated; code search 10/min) were not re-verified for 2026.

## YouTube: transcripts and captions

### Takeaway
YouTube is the hardest target. As of 2026 the caption `baseUrl` (timedtext) in the watch-page player response returns **HTTP 200 with an empty body** unless a valid PO (proof-of-origin) token is attached. Datacenter and cloud IPs are additionally blocked ("Sign in to confirm you're not a bot" / `IpBlocked`). The official Data API `captions.download` requires OAuth from someone **with edit permission on the video**, so it is useless for third-party research. Public Invidious instances are largely dead; Piped is spotty. The workable routes are yt-dlp with a PO-token provider plugin (bgutil), or a paid transcript API. All are fragile.

### Cited Findings
- **[live probe 2026-10-31]** The watch page for `dQw4w9WgXcQ` with a Chrome UA returned `playabilityStatus: OK` and `captionTracks` whose `baseUrl` includes `exp=xpe` and a signature. Fetching that baseUrl (plain and with `&fmt=json3`) returned **200 with 0 bytes**. The legacy `https://www.youtube.com/api/timedtext?v=...&lang=en` also returned 200 with 0 bytes. No bot interstitial appeared from this (non-datacenter) IP.
- YouTube now requires a PO token (generated by BotGuard) on caption requests. Without it timedtext returns an empty 200, not a 403. An Apify actor page dates enforcement for captions to **April 2026**. One vendor claims the ANDROID client is not PO-token-gated; this is unverified. — [DEV Community, 2026](https://dev.to/jamhimself/why-your-youtube-transcript-scraper-started-returning-empty-strings-and-how-to-fix-it-in-2026-20ed); [Prepublish guide](https://prepublish.ai/guides/youtube-timedtext-empty-response); [Apify aurenic actor](https://apify.com/aurenic/youtube-transcript-scraper)
- youtube-transcript-api (v1.2.x) has distinct exceptions: PO token required, `IpBlocked` and `RequestBlocked`. Its documented fix for blocked IPs is proxies or cookies. One user reports `IpBlocked` after about 30 fetches from a home connection. Cloud ranges (AWS, GCP, Azure, Cloudflare Workers) are blocked outright. — [tessl registry: youtube-transcript-api 1.2.0 error docs](https://tessl.io/registry/tessl/pypi-youtube-transcript-api/1.2.0/files/docs/error-handling.md); [DEV Community](https://dev.to/jamhimself/why-your-youtube-transcript-scraper-started-returning-empty-strings-and-how-to-fix-it-in-2026-20ed); [2outube](https://2outube.com/youtube-transcript-api-alternative) (vendor)
- yt-dlp PO Token Guide (last edited **2026-07-12**): the `web` client needs PO tokens for **GVS and Subs** (subtitles). `mweb`, `tv_simply`, `web_music` and `web_safari` need them for GVS only. `android` and `ios` need them for GVS or Player and do not accept cookies. `tv`, `web_embedded` and `android_vr` need none, with caveats (DRM, embeddable-only, no "made for kids"). Recommended providers are **bgutil-ytdlp-pot-provider** (Brainicism, maintained by a yt-dlp maintainer) and yt-dlp-getpot-wpc (browser-based). Tokens are bound to the video ID. — [yt-dlp wiki: PO Token Guide](https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide)
- Official route: `captions.download` needs OAuth scope `youtube.force-ssl` or `youtubepartner` and "requires the user to have permission to edit the video". It costs 200 quota units per call. — [YouTube Data API: captions.download](https://developers.google.com/youtube/v3/docs/captions/download)
- robots.txt **[live probe 2026-10-31]**: `User-agent: *` disallows `/api/`, `/results`, `/get_video_info`, `/feeds/videos.xml`, `/live_chat` and others. Timedtext lives under `/api/`, so it is robots-disallowed for generic crawlers. No AI-specific groups appear at the top.
- Invidious and Piped: public Invidious playback has been broken since Google closed the last workaround (reported **Sept 2024**). Comparison pages dated March 2026 claim about 3 public Invidious instances versus about 15 Piped; this is single-source and unverified. Self-hosting with IPv6 rotation or proxies is the recommended path. — [chipp.in](https://chipp.in/entertainment/you-cant-use-invidious-anymore-to-watch-youtube-videos/); [sumguy.com 2026 status](https://sumguy.com/invidious-piped-redlib-nitter-2026/); [selfhosting.sh](https://selfhosting.sh/compare/invidious-vs-piped/)

### Inferences
- A research CLI should not try to implement BotGuard itself. Realistic options, in order:
  1. Shell out to `yt-dlp --write-subs --write-auto-subs --skip-download` when installed, which inherits its PO-token provider plugins.
  2. Read metadata and descriptions through the Data API v3 with an API key (videos.list costs 1 unit; captions.list gives track names only).
  3. Use a paid transcript provider (Apify actors and similar), which is opt-in.
- Users may also supply browser cookies, but that carries account-ban risk.
- Expect breakage roughly monthly and surface errors honestly. An empty 200 means a missing PO token; `IpBlocked` means the egress IP.

### Gaps
- No official YouTube statement on PO-token enforcement for captions exists. The April 2026 date is from one vendor page.
- Current youtube-transcript-api release and maintainer status for late 2026 (whether it has built-in PO-token support) were not confirmed. I did not fetch its GitHub issues.
- Live Invidious/Piped instance counts as of October 2026 were not verified against api.invidious.io.

## Hacker News: Algolia Search API and Firebase API

### Takeaway
Both APIs are open, keyless and fully usable. HN is the easy case. The HTML site's robots.txt sets `Crawl-delay: 30`, so the APIs are the right route.

### Cited Findings
- **[live probe 2026-10-31]** `hn.algolia.com/api/v1/search?query=rust` returned JSON hits. `hacker-news.firebaseio.com/v0/item/8863.json` returned the item. Neither needed a key.
- Firebase API: the README says "there is currently no rate limit" (with a warning not to hammer it). No rate-limit headers are returned. — [DEV: Firebase vs Algolia](https://dev.to/odeeb/hacker-news-firebase-api-vs-algolia-which-one-to-use-3g2n); [pkg.go.dev hackernews-cli](https://pkg.go.dev/github.com/tamnd/hackernews-cli/hackernews)
- Algolia HN API: third-party sources cite about 10,000 requests/hour/IP and a 1,000-result cap per query. Algolia's own docs only say per-key per-IP limits are configurable, so the figure is unverified. — [DEV 2026 guide](https://dev.to/agenthustler/scraping-hacker-news-in-2026-complete-guide-algolia-api-date-filters-3iep); [Algolia API key restrictions](https://www.algolia.com/doc/guides/security/api-keys/in-depth/api-key-restrictions)
- **[live probe 2026-10-31]** `news.ycombinator.com/robots.txt` contains `User-Agent: *`, `Crawl-delay: 30` and disallows action endpoints (`/vote?`, `/reply?`, `/login`). There are no AI-specific rules.

### Inferences
- Use Algolia for search (`search`, `search_by_date`, `tags=story`/`comment`, `numericFilters`). Use `items/{id}` on Algolia for a full comment tree in one call. Use Firebase for canonical single items. The linked article itself is a separate retrieval problem.

### Gaps
- No official, current documentation of Algolia HN rate limits was found.

## Wikipedia (control)

### Takeaway
Wikipedia is open but increasingly strict about identification. A descriptive User-Agent with contact information gets in. Generic library UAs are refused. Since 2025 the Foundation has rate-limited APIs and points bulk/AI users to paid Wikimedia Enterprise. For a research CLI it remains the reference "cooperative" site.

### Cited Findings
- **[live probe 2026-10-31]** The REST summary endpoint (`/api/rest_v1/page/summary/...`) returned 200 to a browser UA. `/wiki/Python_(programming_language)` with UA `python-requests/2.31` returned **403**. Robots.txt has 35 `User-agent` groups, and grepping found no GPTBot, Claude or CCBot groups.
- In 2025 the WMF reported that 65% of its most expensive traffic came from bots. It started introducing overall API rate limits (around March 2025). Clients that identify themselves (bot-group accounts, well-known clients) get higher limits. The User-Agent policy requires bots to identify themselves, and bots must respect 429 waits. — [IETF 123 MAPRG slides on Wikimedia bot traffic](https://datatracker.ietf.org/meeting/123/materials/slides-123-maprg-bot-traffic-at-wikimedia-measurement-identification-and-response-00); [The Register 2025-04-03](https://www.theregister.com/2025/04/03/wikimedia_foundation_bemoans_bot_bandwidth/); [Phabricator T392904 "Responsible Use of Infrastructure"](https://phabricator.wikimedia.org/T392904); [Phabricator T400119](https://phabricator.wikimedia.org/T400119)
- In November 2025 the WMF urged AI developers to stop scraping and use the paid, opt-in Wikimedia Enterprise API. Human pageviews fell about 8% year on year while AI bot traffic spiked in May-June 2025. — [YourStory](https://yourstory.com/ai-story/wikipedia-urges-ai-use-paid-api-stop-scraping); [AIChief](https://aichief.com/news/wikipedia-tells-ai-firms-to-stop-scraping-and-use-paid-api/)

### Inferences
- The CLI should send a UA like `search-cli/<ver> (<repo URL>; contact)`, use the REST/Action APIs, and back off on 429. This is the template for "legitimate agent" behaviour elsewhere.

### Gaps
- The exact numeric anonymous API rate limit for late 2026 was not found. Whether Wikimedia Enterprise still has a free tier was not confirmed: sources describe it as paid, though historically there was a limited free tier.

## News paywalls: NYT, WSJ, Bloomberg, FT, The Economist and metered sites

### Takeaway
All five front their sites with commercial bot management: NYT and WSJ use **DataDome**, Bloomberg uses **PerimeterX/HUMAN**, and FT and The Economist use **Cloudflare challenges**. A non-browser client gets 401/403 before any paywall logic runs. Robots.txt blocks AI crawlers broadly. NYT, Bloomberg and The Economist also block the *user-triggered* agents (ChatGPT-User, Claude-User, Perplexity-User), and FT blocks Perplexity-User. Since late 2025, NYT, FT, USA Today and 340+ local outlets also block the Internet Archive crawler, so recent Wayback captures of them are largely absent. The legitimate routes for a CLI are RSS feeds, Google News RSS, official APIs (NYT: metadata and abstracts only), and user-supplied subscriber sessions. Litigation (NYT v. OpenAI, Dow Jones v. Perplexity) has no merits ruling yet.

### Cited Findings
**Bot management and status codes [live probe 2026-10-31, Chrome UA via curl]:**
- nytimes.com: 403, `server: DataDome`, `x-datadome: protected`, `x-datadome-riskscore: 0.587`.
- wsj.com: **401**, `server: CloudFront`, `x-datadome: protected`.
- bloomberg.com: 403, `server: Varnish`, sets `_pxhd` cookie (PerimeterX/HUMAN).
- ft.com: 403, `server: cloudflare`, `cf-mitigated: challenge`.
- economist.com: 403, `server: cloudflare`, `cf-mitigated: challenge`.

**robots.txt AI rules [live probe 2026-10-31]:**
- **NYT** has groups for anthropic-ai, Applebot-Extended, Bytespider, CCBot, ChatGPT-User, ClaudeBot, Claude-SearchBot, **Claude-User** (`Disallow: /` confirmed), Google-Extended, GPTBot, Meta-ExternalAgent, OAI-SearchBot, PerplexityBot and Perplexity-User. Both training and user-triggered agents are blocked.
- **Bloomberg** has groups for Google-Extended, CCBot, anthropic-ai, PerplexityBot, GPTBot, ClaudeBot, OAI-SearchBot, Claude-SearchBot, ChatGPT-User, Claude-User, Bytespider, Meta-ExternalAgent and Applebot-Extended.
- **FT** has groups for CCBot, Google-Extended, anthropic-ai, Bytespider, PerplexityBot (`Disallow: /`), Perplexity-User, ClaudeBot (`Disallow: /`), Applebot-Extended and meta-externalagent. ChatGPT-User and Claude-User are not named, which matches FT's licensing deal with OpenAI (2024).
- **The Economist** sends GPTBot and ClaudeBot to `Disallow: /`. Its groups for ChatGPT-User, Claude-User, Claude-SearchBot, OAI-SearchBot, PerplexityBot and Perplexity-User contain only `Allow: /pro`, `/syndication` lines, so it allows marketing pages and implicitly blocks the rest.
- **WSJ** lists ChatGPT-User, GPTBot and OAI-SearchBot in a group with no rules beneath before the next group. The group is empty or the rules are elsewhere; interpret with caution. WSJ licenses to OpenAI through the News Corp deal (2024).

**Paywall type:** these come from training knowledge and press as of 2024-2025; none of the probes reached article bodies.
- WSJ, FT, Bloomberg and The Economist are hard or subscription paywalls. Body text is not delivered server-side to anonymous clients; FT and WSJ also use server-side entitlement checks.
- NYT is metered. Historically the body was present in the HTML with a client-side overlay, which is why archive services worked; the 2026 DataDome front now blocks non-browsers before that point.

**Archives:**
- Since late 2025, publishers including NYT, USA Today and FT block the Internet Archive crawler. Wired counted 241 sites including 23 news organizations. NYT's stated reason is that the Wayback Machine "provides unfettered access to Times content — including by AI companies — without authorization". More than 340 local outlets (USA Today Co., McClatchy) followed by May 2026. — [Nieman Lab 2026-01](https://www.niemanlab.org/2026/01/news-publishers-limit-internet-archive-access-due-to-ai-scraping-concerns/); [Fortune 2026-04-15](https://fortune.com/2026/04/15/why-is-internet-archive-wayback-machine-not-working-news-outlets-block-ai/); [Techdirt 2026-02-13](https://www.techdirt.com/2026/02/13/news-publishers-are-now-blocking-the-internet-archive-and-we-may-all-regret-it/); [WGCU/Nieman 2026-05-22](https://www.wgcu.org/section/science/2026-05-22/over-340-local-news-outlets-are-limiting-the-internet-archives-access-to-their-journalism); [Marketplace 2026-04-21](https://www.marketplace.org/episode/2026/04/21/why-news-sites-are-blocking-the-wayback-machine)
- **[live probe 2026-10-31]** The Wayback availability API for `nytimes.com/2026/10/01/` returned `archived_snapshots: {}` (no capture). CDX queries for `nytimes.com/2026/10/*` and `ft.com/content/*` returned an "Internet Archive: Temporarily Offline" HTML page, while a CDX query for example.com returned 200. This may be an exclusion or a transient outage; it is not conclusive. archive.ph/archive.today timed out (curl code 000) from this host and could not be assessed.

**Feeds and APIs:**
- **[live probe 2026-10-31]** NYT RSS (`rss.nytimes.com/.../HomePage.xml`) returned 200 (37 KB). The Google News RSS search endpoint returned 200 (135 KB). Both give headline, link, date and a short description.
- NYT Developer APIs (Article Search, Top Stories and others) need a free key. Article Search returns metadata such as headline, abstract, snippet, lead_paragraph and web_url, not full text. Quotas are inconsistent across secondary sources: 5/min and 500/day ([Jentic](https://jentic.com/apis/nytimes.com/article-search)) versus 10/min and 4,000/day ([headliner docs](https://headliner.readthedocs.io/en/latest/apis.html)) versus 1,000/day ([rtimes](https://rdrr.io/cran/rtimes/man/rtimes-package.html)). developer.nytimes.com could not be fetched to settle it. The claim that responses carry no full text is from training knowledge and was not re-verified in 2026.

**Litigation:**
- **NYT v. OpenAI/Microsoft** (S.D.N.Y., in the OpenAI MDL): no fair-use ruling and no trial date. Cross-motions for summary judgment were filed in September 2026. Oppositions were filed on 2026-10-23, and replies are reportedly due 2026-11-20. On 2026-01-05 Judge Stein affirmed the order compelling production of 20M de-identified ChatGPT logs. — [aivortex tracker, 2026-10-29](https://www.aivortex.io/legal/ai-case-law/nyt-v-openai/); [Ars Technica-hosted order 2026-01-05](https://cdn.arstechnica.net/wp-content/uploads/2026/01/NYT-v-OpenAI-Order-1-5-26.pdf); [legalaiinsights](https://legalaiinsights.com/risk-digest/nytimes-openai-lawsuit-status). Trackers conflict on whether summary-judgment briefing happened in spring or September 2026.
- **Dow Jones & NY Post v. Perplexity** (S.D.N.Y. 1:24-cv-07984, Judge Failla): Perplexity's venue/transfer motion was denied on 2025-08-21. The case is still in discovery with no merits ruling. In April 2026 Perplexity was ordered to produce seven more months of user-activity logs. Schedules conflict: one tracker gives fact discovery by 2026-06-04 and expert discovery by 2026-09-02, while Inner City Press reports a pre-trial conference on 2026-12-14. — [BakerHostetler case page](https://www.bakerlaw.com/dow-jones-company-inc-v-perplexity-ai-inc/); [Law360](https://www.law360.com/articles/2462537/dow-jones-wins-order-for-more-months-of-perplexity-ai-logs); [Inner City Press](https://innercitypress.com/sdny190afaillaperplexityaidjicp050126.html); [fiund tracker](https://fiund.com/lawsuits/dow-jones-v-perplexity)

### Inferences
- A legitimate research CLI should not attempt to bypass DataDome, PerimeterX or Cloudflare, or a paywall. Beyond being fragile, it lands squarely in the conduct the Perplexity suit targets ("skip the links"). Defensible routes:
  1. Search and discovery via the search provider's snippets, publisher RSS, or Google News RSS.
  2. The NYT Article Search API for abstract and lead paragraph.
  3. A clear "paywalled; open in browser" result with the canonical URL.
  4. Optionally, a user's own logged-in browser session (an extension or browser-automation handoff), so the fetch is the subscriber's own access.
- Note that a user-triggered agent UA (Claude-User, ChatGPT-User, Perplexity-User) is explicitly disallowed by NYT, Bloomberg and The Economist. A CLI that honours robots.txt for its own UA should expect "no" from these sites.
- For older articles, Wayback captures from before late 2025 may still exist for NYT and others. New captures are unlikely for the blocking publishers.
- Metered sites in general (most US regionals, the Atlantic, the WaPo meter) increasingly sit behind the same bot-management vendors. Treat 401/403 with `x-datadome`, `cf-mitigated` or `_px*` as "blocked by bot management", which is distinct from a paywall.

### Gaps
- No current (2026) source verified the server-side vs client-side paywall implementation for each publisher. The paywall-type statements come from training knowledge.
- archive.today status for news in 2026 could not be assessed, because it timed out from this host and no 2026 reporting turned up.
- No official WSJ/Dow Jones or FT public article API for third parties was found. Both offer licensed feeds through commercial agreements (Factiva and others); not verified this session.
- The exact rule text under WSJ's AI-agent group, and the full Bloomberg and FT rules for every agent, were not exhaustively parsed.
