# Social and UGC Platforms: Restrictions on Agentic Retrieval (state as of 2026-10-31)

Scope note: this research ran on a budget of about 15 tool calls. Reddit and X are covered in depth with dated, cited facts. Threads, LinkedIn, Telegram and Substack have partial coverage. Medium, Quora, Instagram, Facebook, Glassdoor and Discord got little or no 2026-dated sourcing; what is known about them is listed under Gaps as unverified background knowledge. Many 2026 sources are scraper or API vendors (voidmob, crawlora, socialcrawl, Apify actors, etc.), which have commercial bias. Each such source is flagged as a vendor.

## Which platforms are retrievable without login, and which serve server-rendered HTML or JSON to anonymous clients?

### Takeaway
By late 2026 anonymous access has collapsed on both major text-UGC platforms. Reddit removed logged-out `.json` (May/June 2026), put old.reddit behind a login (June/July 2026), and is ending RSS on 2026-11-13. X has no anonymous path and no working Nitter. The sources still showing anonymous server-rendered or feed access are Telegram public channels (`t.me/s/<channel>`) and Substack (`/feed` RSS plus the unofficial `/api/v1/posts`, free posts only). The 2026 sources say nothing about Medium RSS, though it is very likely still working.

### Cited Findings
**Per-platform anonymous-access matrix (sourced items only)**
- **Reddit, logged-out `.json`:** returns 403 since late May 2026. A May 2026 r/modnews post announced "the shutdown of unauthenticated .json endpoints"; logged-in access is unaffected — [voidmob (vendor)](https://voidmob.com/blog/how-to-scrape-reddit-2026). Crawlora dates the deprecation to 2026-05-28, with 403s "within days" and no grace period — [Crawlora (vendor)](https://crawlora.net/blog/reddit-json-api-blocked-2026). ppc.land says "around June 2026" — [ppc.land](https://ppc.land/reddit-to-halt-public-data-api-access-in-march-2027-as-14-000-apps-register/). One Apify changelog dates the signed-out JSON cutoff to 2026-10-16, which may be a later tightening — [search summary of Apify changelog](https://apify.com/foxlabs/reddit-subreddit-posts-scraper/changelog.md).
- **Reddit, old.reddit.com:** On 2026-06-30 Reddit said logged-out users would need to log in, rolling out over the following month. Reddit called old Reddit's logged-out experience "a significant source of abusive scraping and automated traffic" and said logged-out browsing on reddit.com was not affected — [RedLens](https://www.redlens.dev/posts/does-old-reddit-still-work-without-logging-in); [noobclaw](https://noobclaw.com/blog/reddit-login-required-2026/). Logged-in old Reddit will be limited to accounts that used it in the previous six months (first announced as 90 days), with moderators exempt — [ppc.land](https://ppc.land/reddit-to-halt-public-data-api-access-in-march-2027-as-14-000-apps-register/).
- **Reddit, www.reddit.com HTML:** Plain HTTP requests can get HTTP 200 carrying a JS challenge or the "You've been blocked by network security" page, with zero posts — [voidmob (vendor)](https://voidmob.com/blog/how-to-scrape-reddit-2026). Redlib developers found www.reddit.com returning an empty client-rendered shell with no comments (Oct 2026) — [redlib #591](https://github.com/redlib-org/redlib/issues/591).
- **Reddit, RSS:** support ends 2026-11-13, per the r/modnews and r/redditdev posts of 2026-09-30 — [ppc.land](https://ppc.land/reddit-to-halt-public-data-api-access-in-march-2027-as-14-000-apps-register/); [voidmob](https://voidmob.com/blog/how-to-scrape-reddit-2026). As of Oct 2026 the feeds "still answer", and some CLI projects route sub, post, user and search through them — [search summary, only-cli/oc release 0.5.2](https://github.com/only-cli/oc/releases/tag/v0.5.2).
- **X:** Nitter's GitHub repository was archived on 2026-09-11 (the wiki shows an archive date of 2026-08-26). XCancel suspended service on 2026-09-14 — [TechCrunch 2026-09-15](https://techcrunch.com/2026/09/15/nitter-and-xcancel-are-dead-again-after-xs-latest-legal-actions/); [Nitter wiki](https://github.com/zedeus/nitter/wiki/Instances).
- **Telegram:** public channels have a no-login web preview at `t.me/s/<channel>`, paginated with `?before=<msg_id>` (and `?after=`). The preview shows text and view counts but not reactions or poll votes. Channels can disable the preview, and private channels are not exposed. Sources disagree on depth: about 20 messages per page versus "latest 500–1,000" reachable — [DEV: sndzdata](https://dev.to/sndzdata/how-to-scrape-a-public-telegram-channel-without-an-account-or-api-key-542j); [DEV: yuhehe](https://dev.to/yuhehe/telegram-scraping-in-python-without-an-api-key-the-public-preview-collector-that-survives-12ef); [clura.ai](https://clura.ai/blog/telegram-scraper) (all vendor or community sources).
- **Substack, RSS:** every publication serves `/feed` with about the 20 most recent posts and no pagination. Paid posts appear as a teaser plus a subscribe prompt — [wprssaggregator](https://www.wprssaggregator.com/substack-rss-feed/); [thedatacollector](https://thedatacollector.substack.com/p/how-to-scrape-substack-newsletters).
- **Substack, `/api/v1/posts`:** an unofficial, undocumented JSON endpoint that needs no authentication. It is rate-limited and paid-only bodies are not returned. Substack has no official public developer API — [thedatacollector](https://thedatacollector.substack.com/p/how-to-scrape-substack-newsletters); [netrows](https://www.netrows.com/blog/best-substack-newsletter-data-apis-2026); [substack-api PyPI](https://pypi.org/project/substack-api/) (that library reads paywalled content you are entitled to by supplying your own session cookies).
- **Threads:** several Apify actors advertise logged-out keyword search and profile and post retrieval. Their methods are undocumented and fragile — [Apify threads-scraper](https://apify.com/aydar_cosmos/threads-scraper); [threads-content-research-agent](https://pkg.go.dev/github.com/Egor01KKK/threads-content-research-agent).
- **Cross-cutting (Cloudflare):** from 2026-09-15, Cloudflare's default settings block "mixed-use" crawlers (search + agent + training) on pages that host ads, unless the site owner opts out. Pay-Per-Crawl is evolving into "Pay Per Use" — [TechCrunch 2026-07-01](https://techcrunch.com/2026/07/01/cloudflares-new-policy-pushes-ai-companies-to-pay-for-publishers-content/); [technology.org](https://www.technology.org/2026/07/03/cloudflare-blocks-mixed-use-ai-crawlers/). This matters for Medium, Substack custom domains and Quora if they sit behind Cloudflare (not verified per platform).

### Inferences
- Rough tiers for a CLI, as of 2026-10-31:
  - **Anonymous and sanctioned-ish:** Telegram `t.me/s/`; Substack `/feed`; Medium RSS (unverified for 2026).
  - **Anonymous but undocumented:** Substack `/api/v1/`; Reddit RSS, until 2026-11-13.
  - **Effectively login- or API-only:** Reddit, X, LinkedIn, Instagram, Facebook, Glassdoor, Discord; Quora presumably.
- RSS and Atom remain a sanctioned route for blog-style platforms (Substack, and presumably Medium), but not for Reddit after 2026-11-13.
- Reddit answers HTTP 200 with a block or challenge page. A CLI must detect blocks by content (zero posts, "blocked by network security" text), not by status code.

### Gaps
- **Medium (2026):** no 2026-dated source found. The only Medium result is from 2023: Medium blocked OpenAI's GPTBot and floated a publisher coalition against AI crawlers — [TechCrunch 2023-09-28](https://techcrunch.com/2023/09/28/medium-hints-at-a-nascent-media-coalition-to-block-ai-crawlers). Unverified background knowledge, all pre-2026: `medium.com/feed/@user` and `/feed/<publication>` RSS exist; `?format=json` returned JSON prefixed with an XSSI guard `])}while(1);</x>`; member-only stories are metered and paywalled; the Scribe frontend (scribe.rip) proxied Medium. Current status of all of these is unknown.
- **Quora:** no 2026 source found. Background knowledge (unverified): Quora shows a login or signup wall after a few page views or scrolls, and much of the content loads client-side.
- **Instagram and Facebook:** no 2026 source found. Background knowledge (unverified): logged-out viewing hits a login wall quickly. Meta's Graph API (Instagram Graph API, Pages API) needs app review and business or creator accounts. Meta Content Library is restricted to vetted researchers. The Bibliogram frontend died around 2022, and Imginn/Picuki-style viewers are unreliable.
- **Glassdoor:** no source found. Background knowledge (unverified): heavy login or contribution walls ("give to get"); no public API since the old partner API was retired.
- **Discord:** no source found. Background knowledge (unverified): there is no anonymous web view of server content. Reading requires a bot token added by a server admin (Bot API), and self-bots and user-token automation violate Discord's ToS.
- **Bot-detection vendors per platform:** not verified in this pass. Reddit's detection signals are unpublished — [voidmob](https://voidmob.com/blog/how-to-scrape-reddit-2026).

## What did Reddit change in 2023–2026, and what anonymous access still works?

### Takeaway
Reddit has moved step by step from paid API (2023) to licensed-only crawling (2024) to closing all anonymous and public machine access (2026). By March 2027 only registered or approved apps and written-agreement partners will have data access. It is litigating aggressively, and a July 2026 SDNY ruling held that bot-detection and CAPTCHA can be DMCA §1201 access controls.

### Cited Findings
**Timeline**
- **2023 to mid-2024:**
  - **June 2023:** API pricing change ended free access for third-party apps — [ppc.land](https://ppc.land/reddit-to-halt-public-data-api-access-in-march-2027-as-14-000-apps-register/); [Wikipedia: Reddit API controversy](https://en.wikipedia.org/wiki/Reddit_API_controversy).
  - **February 2024:** Google data-licensing deal, reported at about $60M/year — [ppc.land](https://ppc.land/reddit-to-halt-public-data-api-access-in-march-2027-as-14-000-apps-register/).
  - **2024-05-17:** OpenAI data-licensing partnership — [ppc.land](https://ppc.land/reddit-to-halt-public-data-api-access-in-march-2027-as-14-000-apps-register/).
  - **Mid-2024:** robots.txt became `User-agent: *` / `Disallow: /`, with access only for partners — [voidmob](https://voidmob.com/blog/how-to-scrape-reddit-2026). Reddit reportedly serves Google a permissive robots.txt and everyone else a prohibitive one — [ppc.land antitrust](https://ppc.land/reddit-faces-antitrust-counterclaims-over-60m-google-only-crawler-deal/).
- **2025:**
  - **June 2025:** Reddit sued Anthropic. A reported March 2026 remand order says the claims rest on access methods and deceptive conduct rather than copyright-equivalent rights. Procedural details are not confirmed from a primary source — [nohacks](https://nohacks.co/blog/what-courts-say-about-blocking-ai-bots); [voidmob](https://voidmob.com/blog/how-to-scrape-reddit-2026).
  - **October 2025:** Reddit sued SerpApi, Oxylabs, AWMProxy and Perplexity in SDNY, alleging they scraped Reddit content via Google SERPs using disguised tools. Perplexity continued after a cease-and-desist — [contentgrip](https://www.contentgrip.com/reddit-sues-perplexity/); [ppc.land](https://ppc.land/reddit-faces-antitrust-counterclaims-over-60m-google-only-crawler-deal/).
- **2026, first half:**
  - **2026-03-25:** App label policy, tied to app registration — [ppc.land](https://ppc.land/reddit-to-halt-public-data-api-access-in-march-2027-as-14-000-apps-register/).
  - **About 2026-05-28:** unauthenticated `.json` shut down — see the matrix above.
  - **2026-06-30:** old.reddit login requirement — see the matrix above.
- **2026-07-31, *Reddit v. SerpApi* (SDNY), motion to dismiss largely denied:**
  - Google's SearchGuard (JS challenges, CAPTCHAs) plausibly counts as a §1201(a) access control, even though humans can see the content.
  - Survived: §1201(a)(1)(A) claims against SerpApi and Perplexity (Perplexity treated as a direct circumventor), the §1201(a)(2) trafficking claim against SerpApi, and civil conspiracy.
  - Dismissed: §1201(b) and unjust enrichment/unfair competition (preempted).
  - The court distinguished the N.D. Cal. *Google v. SerpApi* dismissal.
  - Source: [Loeb & Loeb, Aug 2026](https://www.loeb.com/en/insights/publications/2026/08/reddit-v-serpapi-llc).
- **August 2026:** SerpApi filed antitrust counterclaims over Reddit's Google-only crawler arrangement — [ppc.land](https://ppc.land/reddit-faces-antitrust-counterclaims-over-60m-google-only-crawler-deal/).
- **2026-09-30 announcement** (r/modnews and r/redditdev; reported by TechCrunch):
  - **2026-10-31:** new public API access requests stop.
  - **2026-11-13:** RSS ends.
  - **2026-11-30:** migration bounty registration closes. The $1M program pays $1,000 per migrated app to Devvit, which is TypeScript-based.
  - **2027-01-12:** access begins closing for unregistered apps.
  - **March 2027:** public Data API access closes for all remaining apps, except written agreements.
  - More than 14,000 apps had registered.
  - Source: [ppc.land](https://ppc.land/reddit-to-halt-public-data-api-access-in-march-2027-as-14-000-apps-register/).
- **2026-10-05:** Responsible Builder Policy updated: "You must request access and get explicit approval before accessing any Reddit data through our API." The approval requirement extends to non-commercial use. The User Agreement prohibits scraping "without Reddit's prior written consent" — [voidmob](https://voidmob.com/blog/how-to-scrape-reddit-2026).
- **OAuth rate limit** for approved clients: 100 queries per minute per OAuth client ID, averaged over 10 minutes. Responses carry `X-Ratelimit-Remaining` and `X-Ratelimit-Reset` headers. A user agent of the form `platform:app-id:version (by /u/username)` is required — [voidmob, citing the Reddit Data API Wiki](https://voidmob.com/blog/how-to-scrape-reddit-2026).
- **Data licensing revenue:** Q2 2026 "other revenue" (includes data licensing) was $43M, up 24% YoY, against $804.9M total revenue — [ppc.land](https://ppc.land/reddit-to-halt-public-data-api-access-in-march-2027-as-14-000-apps-register/).

**Redlib (Libreddit successor), October 2026**
- **2026-10-20:** comment and thread pages started returning 403 with the "blocked by network security" HTML, while listings still worked — [redlib #591](https://github.com/redlib-org/redlib/issues/591).
- Contributors concluded the block was path-based, not TLS or header-based. Encoding `/comments` as `%2f` returned 200 (PR #592).
- A logged-in Bearer token also worked.
- Later fixes added GraphQL loading with HTML fallback and a fork using Shreddit endpoints (Oct 23–24). A fresh build was confirmed working on 2026-10-24.
- Further issues: #597 (rate limit exceeded, 2026-10-22), #608 (JSON parse failure on front page, 2026-10-29), and #609 (discussion of forced login) — [redlib issues](https://github.com/redlib-org/redlib/issues).
- Public instances flap: for example, redlib.jeikobu.net was down about 4 hours on 2026-10-21 — [uptime #1415](https://github.com/shindouj/uptime/issues/1415).

### Inferences
- **What still works anonymously as of 2026-10-31:**
  - Reddit RSS, for about two more weeks.
  - Redlib instances, intermittently, via workarounds that Reddit is actively closing.
  - Reddit content surfaced through search-engine results or snippets.
- After *Reddit v. SerpApi*, a CLI that evades Reddit's or Google's bot detection carries non-trivial DMCA §1201 risk (motion-to-dismiss stage only). This is especially true if it uses spoofed user agents, residential proxies or TLS impersonation.
- **Legitimate route:** a registered OAuth app at 100 QPM, used by an individual with their own account. However, new public API access requests close on 2026-10-31 and unregistered access closes from 2027-01-12. A new CLI may be unable to obtain access at all after today.
- Path-based blocking (Redlib #591) shows Reddit targeting specific endpoint patterns, so workarounds have days-long half-lives.

### Gaps
- Exact current status of the OAuth "free tier" for non-commercial personal scripts after the 2026-10-05 policy update. Pre-2026 background: 100 QPM free for OAuth, 10 QPM unauthenticated, $0.24 per 1K calls for large commercial users (2023); not re-verified.
- Primary text of the r/modnews posts was not fetched. RSS end-date confirmation comes from ppc.land and voidmob.
- No confirmation of which bot-detection vendor Reddit uses.

## X/Twitter: API tiers and pricing in 2026; Nitter after guest-account removal; is Grok the only practical route?

### Takeaway
X moved to prepaid pay-per-use credits in February 2026. Fixed Basic and Pro tiers were retired (Basic migrated after 2026-06-01, Pro after 2026-09-01). Reads cost about $0.005 per post. Nitter is dead: the repo was archived on 2026-09-11 after a 2026-08-25 cease-and-desist, and XCancel was suspended on 2026-09-14. For an individual CLI, the realistic sanctioned options are the pay-per-use API (cheap for low volume) or an LLM provider's X search (xAI). The status of the syndication and oEmbed endpoints could not be verified.

### Cited Findings
- **Pay-per-use pricing:**
  - Pay-per-use since February 2026 with prepaid credits in the Developer Console. There is no general free tier — [blotato](https://www.blotato.com/blog/twitter-api-pricing); [socialcrawl](https://www.socialcrawl.dev/blog/x-twitter-api-2026); [postproxy](https://postproxy.dev/blog/x-api-pricing-2026/) (third-party).
  - Rates: about $0.005 per post read, $0.010 per user read, $0.015 per post created, $0.20 per post containing a URL — [twittrapi](https://www.twittrapi.com/blog/x-api-pricing-2026) (third-party).
  - **Official:** from 2026-04-20, writes went from $0.01 to $0.015 per post and URL posts cost $0.20 (summoned replies stay at $0.01). "Owned reads" dropped to $0.001 — [X Developers forum announcement](https://devcommunity.x.com/t/x-api-pricing-update-owned-reads-now-0-001-other-changes-effective-april-20-2026/263025).
  - Monthly read cap: 2M per billing cycle, reportedly raised to 3M in October 2026. Sources conflict — [xpoz](https://www.xpoz.ai/blog/guides/understanding-twitter-api-pricing-tiers-and-alternatives/); [postproxy](https://postproxy.dev/blog/x-api-pricing-2026/).
  - Credits: one report says a one-time $10 voucher for active Free-tier users; another says $20 for new developers who add a card. Sources conflict.
- **Legacy and Enterprise tiers:** Basic (reported at $100 or $200/mo; sources conflict) moved to pay-per-use after 2026-06-01. Pro ($5,000/mo) is deprecated, migrating after 2026-09-01. Enterprise is reportedly from about $42,000/mo — [blotato](https://www.blotato.com/blog/twitter-api-pricing); [getxapi](https://www.getxapi.com/twitter-api-pricing) (third-party).
- **Nitter's death:**
  - 2024 (dated): after X's API restrictions, every Nitter instance had to be backed by a real X account.
  - 2026-08-25: X sent Nitter a cease-and-desist alleging "unlawful use and circumvention" of X's API and session tokens.
  - 2026-09-11: repo archived. 2026-09-14: XCancel suspended "until further notice."
  - Source: [TechCrunch 2026-09-15](https://techcrunch.com/2026/09/15/nitter-and-xcancel-are-dead-again-after-xs-latest-legal-actions/).
  - Before archival, the wiki's session format was `.jsonl` with `auth_token`/`ct0` cookies, created by a browser script — [Nitter wiki: Creating session tokens](https://github.com/zedeus/nitter/wiki/Creating-session-tokens/7e06d8c4b6df31111ccce6a7d2b20fe724964d81).
  - A third-party guide claimed 9 healthy instances on 2026-08-21. That predates the shutdown, so treat it as outdated — [wbcomdesigns](https://wbcomdesigns.com/nitter-alternatives-and-competitors/).

### Inferences
- **Cost for a research CLI:** reading 1,000 posts costs about $5 on pay-per-use. That is affordable for an individual and is the cleanest legitimate route, given the official $0.001 "owned reads" applies only to one's own data.
- Running a self-hosted Nitter with personal session tokens is technically possible from the archived code. However, X's C&D language ("circumvention") plus the *Reddit v. SerpApi* §1201 reasoning makes it legally risky.

### Gaps
- Status of `cdn.syndication.twimg.com/tweet-result`, `publish.twitter.com/oembed` and `syndication.twitter.com` timeline embeds in 2026: no source found. Pre-2026 background (unverified): oEmbed and tweet-result served single public tweets anonymously, which made them useful for fetching a known tweet URL but not for search.
- xAI Grok API "live search" / `x_search` tool pricing and availability in 2026: not verified in this pass.
- Bot detection on x.com (logged-out viewing largely blocked since mid-2023, per background knowledge): not re-verified.

## Medium's paywall and bot handling; Substack paywalls vs free posts; Quora login wall

### Takeaway
Substack is the most CLI-friendly. Free posts are fully readable via HTML, `/feed` RSS (about the 20 latest) and the unofficial `/api/v1/posts`. Paid posts show only teasers unless you supply a subscriber session. No 2026 sourcing was found for Medium or Quora.

### Cited Findings
- **Substack:**
  - `/feed` (about 20 latest, no pagination; paid posts are teasers) — [wprssaggregator](https://www.wprssaggregator.com/substack-rss-feed/).
  - `/api/v1/posts` (unauthenticated, rate-limited, no paid bodies) — [thedatacollector](https://thedatacollector.substack.com/p/how-to-scrape-substack-newsletters).
  - No official API — [netrows](https://www.netrows.com/blog/best-substack-newsletter-data-apis-2026).
  - An unofficial Python client is documented — [nickhagar substack_api](https://www.nickhagar.net/substack_api/).
  - Circumventing paywalls violates ToS — [thedatacollector](https://thedatacollector.substack.com/p/how-to-scrape-substack-newsletters).
- **Medium (2023):** blocked GPTBot and floated a publisher coalition against AI crawlers — [TechCrunch 2023-09-28](https://techcrunch.com/2023/09/28/medium-hints-at-a-nascent-media-coalition-to-block-ai-crawlers).

### Inferences
- For Substack, a CLI should prefer `/feed` (sanctioned) and the HTML of the canonical post URL. It should fall back to `/api/v1/posts?limit=…&offset=…` only for older posts beyond the RSS window, and should honour rate limits.

### Gaps
- Medium 2026 metering and Cloudflare posture, `?format=json` status, RSS status, and Scribe (scribe.rip) health: unverified.
- Quora 2026 login-wall behaviour and any API: unverified. Quora has no public read API (background knowledge, unverified).

## Other platforms: official APIs and alternate frontends

### Takeaway
Meta's Threads API now offers keyword search, but only to approved apps. LinkedIn has enforced against scrapers in court (Proxycurl shut down in July 2025). Telegram is the outlier: it has an official, free MTProto API usable by individuals, plus the anonymous `t.me/s/` preview.

### Cited Findings
- **Threads API:**
  - Keyword search requires `threads_basic` plus the `threads_keyword_search` permission and Meta app approval. It supports `search_type` (TOP/RECENT) and `search_mode` (keyword/TAG); docs updated January 2026 — [Meta: Threads keyword search](https://developers.facebook.com/documentation/threads/keyword-search); [media retrieval](https://developers.facebook.com/documentation/threads/reference/media-retrieval).
  - Reported limit of 500 search queries per rolling 7 days — [socialcrawl (vendor)](https://www.socialcrawl.dev/blog/threads-api) (unverified against Meta).
  - Coverage of the new search and analytics features — [ppc.land](https://ppc.land/meta-introduces-new-search-and-analytics-features-to-threads-api-platform/).
- **LinkedIn:**
  - LinkedIn sued Proxycurl in January 2025, alleging fake accounts that simulated logged-in scraping. Proxycurl shut down on 2025-07-04. The court order requires deletion of the scraped data — [Social Media Today](https://www.socialmediatoday.com/news/linkedin-wins-legal-case-data-scrapers-proxycurl/756101/); [nubela "Goodbye Proxycurl"](https://nubela.co/blog/goodbye-proxycurl/); [startuphub](https://www.startuphub.ai/ai-news/startup-news/2025/the-1-linkedin-scraping-startup-proxycurl-shuts-down).
  - Scraping genuinely public data is not per se unlawful under *hiQ v. LinkedIn*, but it still breaches LinkedIn's terms and is subject to GDPR and CCPA — [linkedapi guide (vendor)](https://linkedapi.io/guides/proxycurl-alternatives).
- **Telegram:** the `t.me/s/` web preview is described in the anonymous-access matrix above. Some channels disable it.

### Inferences
- Threads is the only Meta property with a realistic individual-developer read API, and it still needs app review. Instagram and Facebook public content is effectively unavailable to a CLI without Meta approval.
- Telegram is the best-behaved target. Use `t.me/s/` for anonymous reads of public channels, or the official MTProto API with a user's own `api_id`.

### Gaps
- **Telegram MTProto API terms** (`api_id` from my.telegram.org, flood-wait limits): not fetched this pass. Background knowledge: free, requires a phone-number account; unverified for 2026.
- **Instagram Graph API, Facebook Pages API and Meta Content Library 2026 terms:** not fetched.
- **Discord API:** bot token needed and the bot must be invited to the server; not fetched.
- **Glassdoor:** no API or anonymous route found.
- **Alternate-frontend status not verified for 2026:**
  - Scribe (Medium)
  - ProxiTok (TikTok; background: largely broken since 2023)
  - Rimgo (Imgur)
  - Bibliogram (Instagram; background: discontinued around 2022)
  - Quetre (Quora)
- **Robots.txt texts:** not fetched directly for LinkedIn, Instagram, Facebook, Quora, Medium, Glassdoor or Threads.
