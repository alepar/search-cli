# Changelog

## Unreleased

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