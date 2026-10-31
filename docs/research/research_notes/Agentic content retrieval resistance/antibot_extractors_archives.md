# Anti-bot infrastructure, hosted extractors, and archive fallbacks (as of 2026-10-31)

Context: search-cli extract chain = local wreq fetch (Chrome136 TLS/HTTP2 emulation + readability) -> Jina Reader -> Firecrawl /scrape -> Browserless. Research date 2026-10-31. Many sources are vendor marketing or secondary blogs; this is flagged inline. Roughly 22 tool calls were spent; several sub-questions remain gaps (listed per section).

## 1. Bot-protection vendors, their signals, and how far TLS impersonation gets

### Takeaway
Cloudflare has moved from "block bad bots" to an AI-crawler policy layer: default AI-crawler blocking for new domains (2025-07-01), Pay Per Crawl via HTTP 402, and since 2026-07-01 a Verified-bot taxonomy that folds in Web Bot Auth "signed agents" (Direct vs Intermediary). Commercial vendors (Cloudflare, Akamai, DataDome, HUMAN/PerimeterX, Kasada) score TLS (JA3/JA4), HTTP/2 framing, header shape, IP reputation and injected-JS/behavioral signals together; TLS impersonation alone (wreq, curl_cffi) clears only the weaker configurations, and even paid unblocker APIs averaged only ~66% on protected sites in an independent-ish Sept 2026 benchmark.

### Cited Findings
**Cloudflare policy timeline**
- July 2024: Cloudflare gave customers a one-click option to block AI crawlers; over 1 million customers enabled it. On 2025-07-01 Cloudflare began blocking known AI crawlers by default for new domains (existing customers opt in) — [Computerworld](https://www.computerworld.com/article/4015341/cloudflare-offers-to-make-ai-pay-to-crawl-websites.html); [AlternativeTo](https://alternativeto.net/news/2025/7/cloudflare-blocks-ai-crawlers-by-default-and-launches-pay-per-crawl-for-publishers); [String stats page](https://usestring.ai/web-scraping-statistics)
- Pay Per Crawl (launched 2025-07-01, private beta) uses HTTP 402 Payment Required; a practitioner guide says non-allow-listed bots get a 402 with a `crawler-price` header (e.g. `crawler-price: USD 0.02`). A 2026 article says it is still private beta with no GA timeline — [Engadget](https://engadget.com/ai/cloudflare-experiment-will-block-ai-bot-scrapers-unless-they-pay-a-fee-121523327.html); [godberrystudios (2026)](https://godberrystudios.com/posts/cloudflare-pay-per-crawl-http-402-scrapers-2026/)
- "AI Crawl Control" is reported as the product umbrella containing Pay Per Crawl; GoDaddy announced (2026-04-07) integrating it into its hosting platform, citing Web Bot Auth to allowlist trusted agents. Not confirmed from a Cloudflare primary source in this research — [pulse.bot](https://www.pulse.bot/ai/news/cloudflare-and-godaddy-ink-partnership-to-rein-in-ai-agents-reshaping-web-traffic-384cc970-2f4a-4bb3-a404-5c59e08e79c4/)
- Cloudflare-vs-Perplexity: on 2025-08-04 Cloudflare published "Perplexity is using stealth, undeclared crawlers to evade website no-crawl directives", alleging rotation through IPs/ASNs outside Perplexity's declared ranges and a generic Chrome UA, across tens of thousands of domains and millions of requests/day. Cloudflare de-listed Perplexity as verified and added signatures to its managed AI-crawler block rule (available to free plans too). Perplexity called it a "sales pitch" and argued user-initiated assistants differ from crawlers — [Cloudflare blog (author page)](https://blog.cloudflare.com/author/reid); [TechRepublic](https://www.techrepublic.com/article/news-cloudflare-accuses-perplexity-stealth-crawling-violations/); [PC Gamer](https://www.pcgamer.com/software/ai/cloudflare-calls-out-perplexity-for-hiding-crawling-activity-as-ai-bot-scrapes-websites-that-explicitly-disallow-it-perplexity-responds-by-calling-them-more-flair-than-cloud)
- Verified bots / signed agents (Cloudflare docs, last updated 2026-10-08): "Signed agents are now Verified." From 2026-07-01 the distinction is a BotBase metadata field: **Direct** (single operator, own infra) vs **Intermediary** (agentic service operated by many end users; "transitive trust"). Cloudflare is testing the `Forwarded` header (RFC 7239) to pass end-user info. Verified bots must identify via a Web Bot Auth signature, a published IP list with stable UA, or reverse DNS; must obey robots.txt / crawl-delay, keep reasonable rates, not evade preferences. Application via BotBase submission form in the dashboard; rejections give no reason — [Cloudflare docs: signed agents](https://developers.cloudflare.com/bots/concepts/bot/signed-agents/)
- Original signed-agents cohort (2025) reportedly: ChatGPT agent, Goose (Block), Browserbase, Anchor Browser — [Crawlbase](https://crawlbase.com/blog/web-bot-auth-signed-agents/); [WorkOS](https://workos.com/blog/web-bot-auth-identify-ai-agents) (secondary)
- AWS WAF also labels requests with a valid Web Bot Auth signature as verified (with invalid/expired/unknown-key variants) — [WorkOS](https://workos.com/blog/web-bot-auth-identify-ai-agents) (secondary)
- Cloudflare traffic stats cited by String: AI bots other than Googlebot = 4.2% of HTML requests in 2025; AI "user action" crawling grew >15x in 2025; 52% of crawler requests were for training as of June 2026; Anthropic crawl-to-referral 38,000:1 and OpenAI 1,091:1 (July 2025); 17% of Cloudflare site owners restrict AI training vs <1% blocking search (Sept 2026) — [String stats](https://usestring.ai/web-scraping-statistics) (aggregator of Cloudflare Radar figures)

**Detection signals (Akamai, DataDome, HUMAN/PerimeterX, Kasada, Imperva)**
- 2026 practitioner write-ups: anti-bot vendors layer JA3/JA4 TLS fingerprint + HTTP/2 fingerprint (SETTINGS frame, WINDOW_UPDATE, pseudo-header order — the "Akamai HTTP/2 fingerprint") + JS-level signals + IP reputation, and score cross-signal consistency. Matching JA3 but not JA4 is called a "wrong-shape Chrome" signal; a client can match Chrome TLS but fail on HTTP/2 if the library uses its own HTTP/2 stack — [ProxyLabs JA4 2026](https://proxylabs.app/blog/ja4-tls-fingerprinting-2026); [krowdev bot detection 2026](https://krowdev.pages.dev/article/bot-detection-2026.md); [Scrappey HTTP/2 fingerprinting](https://scrappey.com/qa/anti-bot/what-is-http2-fingerprinting). Note: one source attributes HTTP/2 framing to "JA4H"; per the FoxIO JA4+ spec JA4H is an HTTP request/header fingerprint, so that attribution is likely wrong.
- DataDome injects a JS payload monitoring browser behavior; Akamai combines static signatures with behavioral scoring from injected JS (sensor data) — [DEV Community 2026](https://dev.to/agenthustler/how-anti-bot-systems-detect-scrapers-in-2026-and-how-to-get-past-them-5fpp)
- Akamai "flags DC IPs aggressively"; residential proxies recommended (practitioner claim, not measured) — [DEV Community](https://dev.to/vhub_systems_ed5641f65d59/how-to-bypass-akamai-bot-detection-in-2026-curl-cffi-residential-proxies-5h3k)
- Anecdotal: a single JA4 fingerprint seen across ~10,000 IPs within 5 minutes is flagged as a bot farm (unverified threshold) — [ProxyLabs](https://proxylabs.app/blog/ja4-tls-fingerprinting-2026)

**How far TLS impersonation gets**
- curl_cffi README itself says TLS fingerprinting alone is not enough for modern bot protection (from sponsor text in README). Latest versions: 0.15.0 (2026-04-03), 0.16.x — [curl_cffi GitHub](https://github.com/lexiforest/curl_cffi); [PyPI 0.16.0](https://pypi.org/project/curl-cffi/0.16.0/)
- Vendor Q&A (updated 2026-05-26): curl_cffi does not execute JS; "tends to work" against Cloudflare without Turnstile, DataDome XHR endpoints, medium-strength Akamai; fails on Cloudflare Turnstile/managed challenge, Akamai sensor.js, F5 Shape; an outdated profile (e.g., Chrome 120 in 2026) is itself suspicious — [Scrappey](https://scrappey.com/qa/web-scraping-apis/what-is-curl-cffi) (vendor claim, untested)
- wreq (Rust, BoringSSL) emulates TLS + JA3/JA4 + HTTP/2 fingerprints; profiles in wreq-util list Chrome100–Chrome137 (a third-party listing claims up to Chrome147 in newer releases); most Chrome profiles share the same TLS/HTTP2 config and differ only in UA. Older README (as rquest 5.2.0) cautioned that JA3/JA4/Akamai fingerprint strings alone cannot fully simulate a browser. Mixing openssl-sys and boring-sys can segfault — [wreq README (docs.rs)](https://docs.rs/crate/wreq/latest/source/README.md); [wreq-util](https://docs.rs/wreq-util)
- No public GitHub issue or benchmark was found that measures wreq specifically against Cloudflare/DataDome in 2026.

**Measured success of unblocker APIs on protected sites (benchmarks)**
- String "Web Data Frontier Benchmark" (published 2026-09-16; 16 APIs, 100 bot-protected sites, 5 attempts each, success = body contains real page text): field average 66.6% (median 70.5%, range 36.4–97.0%); only 1 of 16 APIs > 90%. Per vendor field average: DataDome 57.6% (lowest API 4.2%), AWS WAF 58.5%, Akamai 65.0% (lowest 28.0%), Cloudflare 65.6% (lowest 8.0%), Kasada 77.1%, PerimeterX 78.6%. 16.6% of failures were "false successes" (200 status without the real page). temu.com defeated every API. Average up from 62.7% (July 2026). Caveat: String ran it and ranked itself first (97.0%); Scrapfly second (86.2%); ScrapingAnt last (36.4%). Vendors advertised 98–99.99% — [String](https://usestring.ai/web-scraping-statistics)
- Proxyway (test Oct 2025, published 2025-12-03): G2 (DataDome) averaged 36.63% across 11 APIs; Leboncoin (DataDome) 63.83% — [String](https://usestring.ai/web-scraping-statistics); [ScrapeUnblocker](https://www.scrapeunblocker.com/post/web-scraping-api-benchmark-2026-success-rates)
- Vendor self-benchmarks claim much higher: ZenRows 100% on DataDome/Cloudflare/Akamai; Scrapfly Aug 2026 DataDome benchmark Scrapfly 99%, Firecrawl 99%, ScraperAPI 97% — [ZenRows](https://www.zenrows.com/blog/best-web-scraping-api-for-ai-agents); [Scrapeway DataDome](https://scrapeway.com/anti-bot-services/datadome)

### Inferences
- For search-cli, the wreq hop should be expected to succeed on unprotected and lightly protected (Cloudflare "Bot Fight Mode" off / no challenge) pages but to fail on any page serving a JS challenge/Turnstile, Akamai sensor, DataDome device check, or Kasada PoW. Failures will frequently come back as HTTP 200/403 challenge pages, so content validation (not status code) is required — the String "false success" rate (16.6%) applies equally to a local fetcher.
- Keeping the emulation profile current matters (Chrome136 is roughly 6+ months old by late 2026); a stale profile is itself a signal.
- Running from a datacenter/cloud IP compounds failures regardless of TLS fidelity; a home residential IP is a material advantage for a CLI run on a laptop.
- The sanctioned path through Cloudflare for an agent is Web Bot Auth signing + Verified (Intermediary) registration, not better impersonation; a personal CLI is unlikely to qualify, but hosted browser providers that sign (Browserbase, Anchor) do.

### Gaps
- No primary-source detail found on Kasada's proof-of-work or Imperva (Thales) Advanced Bot Protection signals; HUMAN/PerimeterX signals only via benchmark numbers.
- No Cloudflare primary doc fetched for Super Bot Fight Mode / Turnstile specifics or AI Crawl Control naming.
- No independent measurement of pure TLS-impersonation clients (wreq/curl_cffi/curl-impersonate) vs real browsers vs residential IPs.

## 2. Sanctioned agent-identification and preference standards

### Takeaway
Web Bot Auth (HTTP Message Signatures, RFC 9421, with a key directory) is the de facto agent-identity mechanism, deployed by Cloudflare and AWS WAF, with an IETF webbotauth WG chartered but the architecture draft not clearly adopted. Preference signaling is split between Cloudflare's Content Signals Policy (robots.txt extension, deployed on millions of domains) and the IETF aipref vocabulary, which as of Aug 2026 is still a non-consensus working draft past its milestone date.

### Cited Findings
- Web Bot Auth architecture draft: draft-meunier-web-bot-auth-architecture by Thibault Meunier (Cloudflare), "HTTP Message Signatures for automated traffic Architecture"; -00 announced April 2025; revision -05 exists. Built on RFC 9421 — [IETF HTTP WG list, Apr 2025](https://lists.w3.org/Archives/Public/ietf-http-wg/2025AprJun/0106.html); [datatracker bibxml -05](https://datatracker.ietf.org/doc/bibxml3/draft-meunier-web-bot-auth-architecture-05.xml)
- IETF webbotauth WG charter: standardize methods for cryptographically authenticating non-browser clients and providing operator info to websites; architecture draft listed as candidate input. At IETF 124 (Nov 2025) adoption was still an open question; no confirmation of adoption found — [charter](https://www.ietf.org/ietf-ftp/charter/charter-ietf-webbotauth-00-01.txt); [IETF 124 slides](https://datatracker.ietf.org/meeting/124/materials/slides-124-webbotauth-architecture-majormeunier-01)
- Companion drafts: HTTP message signatures key directory and a registry draft — [registry draft](https://thibmeu.github.io/http-message-signatures-directory/registry-improvements/draft-meunier-webbotauth-registry.txt)
- Verification proves operator identity only, not authorization or intent — [WorkOS](https://workos.com/blog/web-bot-auth-identify-ai-agents)
- Content Signals Policy (Cloudflare, launched Sept 2025 per coverage; date not confirmed from primary): three signals in robots.txt — `search`, `ai-input` (RAG/grounding/live answers), `ai-train`; e.g. `Content-Signal: search=yes, ai-train=no`. Cloudflare managed robots.txt serves `search=yes, ai-train=no` with no ai-input default; managed robots.txt enabled on >3.8M domains. Compliance voluntary — [Cloudflare blog](https://blog.cloudflare.com/content-signals-policy/); [Search Engine World](https://www.searchengineworld.com/cloudflare-enters-the-robots-txt-fray-with-a-content-signals-policy-for-ai-bots)
- IETF aipref: WG Active (chairs Mark Nottingham, Suresh Krishnan; AD Mike Bishop). Charter: standards-track vocabulary + attachment mechanism (draft-ietf-aipref-attach). Milestones targeted August 2026 for both. Latest found draft-ietf-aipref-vocab-07 (2026-08-19) still says contents do not reflect WG consensus. Open questions (opinion, Apr 2026): definition of "search", whether to add a RAG/grounding category. Individual draft-zehta-aipref-parameters-00 (2026-08-06) — [aipref charter](https://datatracker.ietf.org/wg/aipref/charter/); [vocab-07 mirror](https://ftp.kaist.ac.kr/ietf/draft-ietf-aipref-vocab-07.xml); [zehta draft](https://ftp.kaist.ac.kr/ietf/draft-zehta-aipref-parameters-00.txt)
- HTTP Archive: GPTBot named in robots.txt of 4.5% of desktop sites in 2025 (2.9% in 2024) — [String stats](https://usestring.ai/web-scraping-statistics)

### Inferences
- A user-directed CLI fetch maps to Content Signals `ai-input`, which Cloudflare deliberately leaves unset by default; honoring robots.txt is a choice search-cli can make (Jina offers `X-Robots-Txt`).
- Signing requests with Web Bot Auth requires a published key directory and (for Cloudflare benefit) registration; not realistic for an individual CLI user, but worth tracking.

### Gaps
- No sources gathered for ai.txt (Spawning) or llms.txt adoption numbers/status.
- Current exact revision/expiry of the Web Bot Auth drafts and whether webbotauth adopted them by Oct 2026 not confirmed (one unverified report of drafts published 2026-07-14).

## 3. Hosted extraction providers

### Takeaway
Jina Reader is cheap/free but explicitly does not circumvent anti-bot defenses; Firecrawl offers auto/enhanced proxies (5 credits historically, possibly free since Aug 2026 — unverified) but blocks major social platforms; Browserless meters browser time, proxy MB and CAPTCHA solves in units. The "unblocker" tier (Bright Data, Zyte, ScrapingBee, ScraperAPI) prices by difficulty, from ~$0.06 to ~$16 per 1,000 pages. Search-API extractors (Exa, Tavily, Linkup) are cheap ($1/1k or less) but not positioned as anti-bot bypass.

### Cited Findings
**Jina Reader (r.jina.ai)**
- Rate limits: no key 20 RPM (per IP); free and paid key 500 RPM; premium 5,000 RPM; s.jina.ai search blocked without key. Avg latency 7.9 s. Billed by output tokens; new keys get 10M free tokens; failed requests not charged; tokens shared across Jina APIs — [jina.ai/reader](https://jina.ai/reader/); [Jina pricing (zh)](https://jina.ai/zh-CN/api-dashboard/pricing/). Older (2024) aggregators list 200 RPM keyed — outdated.
- Headers: `X-Engine` (default headless browser; `direct` = plain HTTP, fastest; `cf-browser-rendering` experimental), `X-Proxy` (country / `auto` / `none`), `X-Proxy-Url` (bring your own proxy), `X-Set-Cookie` (disables cache), `X-No-Cache`, `X-Timeout`, `X-Respond-With` (e.g. `readerlm-v2`; jina-ocr-v1 costs 40x tokens), `X-Robots-Txt` — [jina.ai/reader](https://jina.ai/reader/)
- Policy: Reader "does not actively circumvent or bypass any website defense mechanisms, anti-bot systems, or access controls"; paid keys "do not grant access to additional websites" — [jina.ai/reader](https://jina.ai/reader/)

**Firecrawl /scrape**
- Proxy modes: `basic` (1 credit), `enhanced` (5 credits/request), `auto` (default; basic then enhanced retry, 5 credits only if enhanced needed) — [Firecrawl stealth mode docs](https://docs.firecrawl.dev/features/stealth-mode)
- Conflict: Proxidize claims Firecrawl changelog says the 5-credit enhanced charge stopped on 2026-08-17; not verified against Firecrawl — [Proxidize](https://proxidize.com/blog/firecrawl-pricing/)
- Site restrictions: returns "This website is no longer supported" (contact help@firecrawl.com to activate) for Instagram, YouTube, TikTok and other social platforms; Reddit/LinkedIn not confirmed in this research; competitor sources — [ScrapeCreators](https://scrapecreators.com/blog/firecrawl-social-scraping-restrictions); [Scrape.do](https://scrape.do/compare/firecrawl-vs-scrapedo)
- Scrapfly's Aug 2026 DataDome benchmark lists Firecrawl at 99% (vendor-run) — [Scrapeway](https://scrapeway.com/anti-bot-services/datadome)

**Browserless**
- Units: 1 unit per 30 s of browser time; residential proxy 6 units/MB; datacenter proxy 2 units/MB; each CAPTCHA solve attempt 10 units (charged on failure too); applies to BrowserQL, BaaS and REST — [Browserless unit consumption](https://docs.browserless.io/overview/unit-consumption)
- Plans: free 1,000 units/month, 2 concurrent browsers (includes BrowserQL editor, automatic captcha solving, residential proxies); Prototyping $25 / 20,000 units (G2, 2026-04-29); a third-party cites Starter 180k units $140/month (unverified) — [Browserless pricing](https://browserless.io/pricing); [G2](https://www.g2.com/products/browserless/pricing)

**Unblocker APIs**
- Zyte API: pay-as-you-go $0.13–$1.27 per 1,000 HTTP responses and $1.01–$16.08 per 1,000 browser-rendered, across 5 site-difficulty tiers; at $500/month commitment $0.06–$0.61 (HTTP) and $0.48–$7.68 (browser) — [Zyte pricing](https://www.zyte.com/pricing); [Zyte dev pricing](https://dev.zyte.com/pricing)
- Bright Data Web Unlocker: mid-2026 restructure to Free/PAYG/Scale/Enterprise with 5,000 free results/month; PAYG ~$1.50/1k, down to ~$1.00/1k at Scale ($499/month); success-only billing; sources conflict (another says ~$3/1k) — [usagepricing.com (2026-07-14)](https://usagepricing.com/blueprint/activity/bright-data-2026-07-14-scraping-api-free-scale); [use-apify.com](https://use-apify.com/blog/bright-data-pricing-guide-2026). Bright Data cites Scrape.do benchmark 98.87% avg (self-promotional) — [Bright Data](https://brightdata.com/blog/web-data/best-web-scraping-apis)
- ScrapingBee: plans $49–$599/month; third-party estimate ~$6.60/1k for JS + stealth/premium requests; official stealth_proxy multiplier not confirmed (commonly cited 75 credits — unverified) — [costbench](https://costbench.com/compare/bright-data-vs-scrapingbee/)
- ScraperAPI: 97% on DataDome in Scrapfly's vendor-run benchmark; pricing/credit multipliers not found — [Scrapeway](https://scrapeway.com/anti-bot-services/datadome)

**AI-search extractors**
- Exa /contents: $1 per 1,000 pages; $10 free credits/month — [TinyFish Exa pricing](https://www.tinyfish.ai/blog/exa-pricing)
- Tavily /extract: basic 1 credit per 5 successful URLs, advanced 2 credits per 5; credits $0.005–$0.0075 on monthly plans (≈$1–$1.50 per 1,000 basic URLs); free 1,000 credits/month — [Tavily API credits](https://docs.tavily.com/documentation/api-credits.md)
- Linkup fetch: $0.001–$0.006 per fetch — [codenote comparison](https://codenote.net/en/posts/tavily-alternatives-cost-comparison-search-extract-api/)
- Parallel Extract: per-page price not found; free $80 at signup + $5/month — [Parallel (Tavily alternatives)](https://parallel.ai/articles/tavily-alternatives)

### Inferences
- In search-cli's chain, Jina adds little on hard anti-bot targets (by its own policy it gives up), but its default browser engine helps on JS-rendered pages that wreq+readability cannot handle; `X-Engine: direct` would duplicate the local hop.
- Firecrawl `auto` proxy is the cheapest meaningful escalation; Browserless is the only hop that can execute challenges and solve CAPTCHAs, but at 10 units/solve plus residential bandwidth it should stay last.
- Neither Jina nor Firecrawl will help on social platforms (policy blocks), so domain-specific routing (e.g., Reddit -> official API or archive) is needed rather than more fallbacks.

### Gaps
- Declared site restrictions for Exa, Tavily, Parallel, Linkup, Zyte, Bright Data (Bright Data is known to have KYC-gated targets) not researched.
- No independent success-rate data for Jina, Exa, Tavily, Linkup on protected sites; String's benchmark did not include them.
- ScraperAPI pricing and ScrapingBee stealth multiplier unconfirmed.

## 4. Archives and caches as fallbacks

### Takeaway
Google cache is gone (Feb–Sept 2024) and points to the Wayback Machine; Wayback's CDX/availability APIs work but are informally limited to ~15 req/min/IP with increasing 429s, and major publishers (Reddit since Aug 2025) exclude it. archive.today has no API and CAPTCHA/DNS friction. Common Crawl's index plus WARC byte-range fetch is a viable, unthrottled-ish but stale (monthly) snapshot source.

### Cited Findings
- Google cache: links removed from results; Search Liaison confirmed removal 2024-02-01; `cache:` operator confirmed dead Sept 2024; "About this result" now links to Wayback Machine — [Search Engine Journal](https://www.searchenginejournal.com/google-removes-cache-search-operator-documentation/528022/); [Search Engine Roundtable](https://seroundtable.com/google-cache-dead-38112.html)
- Reddit (2025-08-11): restricted Wayback Machine to crawling Reddit's homepage only — no posts, comments, subreddits or profiles — citing AI companies harvesting archived Reddit content; IA's Mark Graham said discussions were ongoing — [The Verge via Nieman Lab](https://www.niemanlab.org/reading/reddit-will-block-the-internet-archive/); [SiliconANGLE](https://siliconangle.com/2025/08/11/reddit-says-blocking-internet-archive-stop-sneaky-ai-scrapers-accessing-content/); [9to5Mac](https://9to5mac.com/2025/08/12/reddit-blocks-non-profit-wayback-machine-from-archiving-the-site/)
- Wayback CDX rate limits: no published limit; community reports >~15 req/min/IP triggers 429/503; suggested pace 1 req/s; IA added protections against "waves of high-volume automated traffic" that can 429 real users; 504s on recent date ranges reported — [Ciro Santilli](https://cirosantilli.com/wayback-machine-rate-limit); [waybackcollector](https://beta.pkg.go.dev/github.com/ctoyan/waybackcollector); [Kunal Ganglani 2026](https://www.kunalganglani.com/blog/wayback-machine-rate-limiting-mini-wayback)
- archive.today/archive.ph: no official API; with Cloudflare 1.1.1.1 DNS it fails to resolve or loops on CAPTCHA (reported "resolved" Aug 2024, recurring per Feb 2026 comment) — [ardislu gist](https://gist.github.com/ardislu/b2f2b4b439c5da2f7ccb6bb42e7a8882); [ArchiveTeam wiki](https://wiki.archiveteam.org/index.php/Dealing_with_Cloudflare)
- Common Crawl: roughly monthly crawls on data.commoncrawl.org / S3; CDX index server lookup by URL, then HTTP byte-range fetch of a single WARC record; columnar Parquet index for bulk — [ccrawl-cli docs](https://pkg.go.dev/github.com/tamnd/ccrawl-cli@v0.5.1) (third-party)

### Inferences
- For a CLI, a Wayback availability lookup (`archive.org/wayback/available?url=`) plus `id_` raw-snapshot fetch is the cheapest fallback for static content; respect ~1 req/s and treat 429 as terminal for that run.
- Common Crawl is useful for evergreen pages but snapshots can be weeks old and CCBot is widely disallowed in robots.txt (so protected/AI-hostile sites are under-represented).
- Archives do not help with Reddit after Aug 2025.

### Gaps
- Bing cache status not researched (Bing's cached-page link reportedly also disappeared ~2024–2025; unconfirmed).
- Official Wayback availability API docs, and Common Crawl official index API docs (index.commoncrawl.org) not fetched; CCBot robots.txt block rates not found.
- Whether other publishers (e.g., news outlets) excluded Wayback in 2025–2026 not researched.
