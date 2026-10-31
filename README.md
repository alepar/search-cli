<div align="center">

# Search CLI — Web Search for AI Agents

**One binary, 14 providers, 14 modes, rank-fused results. The web search tool your AI agent is missing.**

<br />

[![Star this repo](https://img.shields.io/github/stars/paperfoot/search-cli?style=for-the-badge&logo=github&label=%E2%AD%90%20Star%20this%20repo&color=yellow)](https://github.com/paperfoot/search-cli/stargazers)
&nbsp;&nbsp;
[![Follow @longevityboris](https://img.shields.io/badge/Follow_%40longevityboris-000000?style=for-the-badge&logo=x&logoColor=white)](https://x.com/longevityboris)

<br />

[![License: MIT](https://img.shields.io/badge/License-MIT-blue?style=for-the-badge)](LICENSE)
[![Build](https://img.shields.io/github/actions/workflow/status/alepar/search-cli/ci.yml?style=for-the-badge&logo=github)](https://github.com/alepar/search-cli/actions)

---

A single Rust binary that aggregates Brave, Serper, Exa, Linkup, Kagi, Jina, Firecrawl, Tavily, SerpApi, Perplexity, Parallel, xAI, and more into one search interface. Built for AI agents from day one: structured JSON, semantic exit codes, self-describing `agent-info`, reciprocal rank fusion across providers, and a `usage` command that reports remaining API credits.

[Install](#install) | [How It Works](#how-it-works) | [Features](#features) | [Providers](#providers) | [Contributing](#contributing)

</div>

## Why This Exists

Every search API is good at something different. Brave has its own 35-billion page index. Serper gives you raw Google results plus Scholar, Patents, and Places. Exa does neural/semantic search. Perplexity gives AI-synthesized answers with citations. Jina reads any URL into clean markdown. Firecrawl renders JavaScript-heavy pages. xAI searches X/Twitter.

You shouldn't have to wire up each one separately, handle their different response formats, or manage rate limits. `search` does the plumbing: you pick the mode (the CLI never guesses intent), it fans out to that mode's providers in parallel and fuses the results with reciprocal rank fusion. A URL that three engines independently return outranks any single engine's top hit — the fastest provider no longer wins by default, the most-agreed-on result does.

```bash
search "CRISPR gene therapy breakthroughs"
```

That's it — a plain `search "query"` runs a general multi-provider web search and merges the results in under 2 seconds. You stay in control of routing: pick a mode with `-m` or specific providers with `-p` (run `search agent-info` for the full map). The CLI never guesses intent from your query.

## Install

**One-liner (macOS / Linux x86_64, recommended):**
```bash
curl -fsSL https://raw.githubusercontent.com/alepar/search-cli/master/install.sh | sh
```

**Prebuilt binaries:** download a `search-<target>.tar.gz` from
[GitHub Releases](https://github.com/alepar/search-cli/releases) and check it
against the release's `SHA256SUMS`.

**From source (Cargo):**
```bash
cargo install --git https://github.com/alepar/search-cli
```

The `agent-search` crate on crates.io is the upstream
[paperfoot/search-cli](https://github.com/paperfoot/search-cli) release, not
this fork.

Binary size is ~6 MB. Startup is ~2 ms. Memory is ~5 MB. No Python, no Node, no Docker.

## Quick Start

```bash
# Set your API keys (any combination works -- even just one)
search config set keys.brave YOUR_BRAVE_KEY
search config set keys.serper YOUR_SERPER_KEY
search config set keys.exa YOUR_EXA_KEY

# Or use environment variables
export SEARCH_KEYS_BRAVE=YOUR_KEY
export SEARCH_KEYS_EXA=YOUR_KEY

# Search
search "your query here"
```

## How It Works

```
                          ┌─────────────┐
                          │ Query + -m  │  you pick the mode —
                          └──────┬──────┘  no intent guessing
                                 │
                    ┌────────────┼────────────┐
                    ▼            ▼            ▼
              ┌──────────┐ ┌──────────┐ ┌──────────┐
              │  Brave   │ │  Serper  │ │   Exa    │  parallel fan-out
              └────┬─────┘ └────┬─────┘ └────┬─────┘  via tokio::JoinSet
                   │            │            │
                   └────────────┼────────────┘
                                │
                         ┌──────▼──────┐
                         │ Rank fusion │  dedup + reciprocal rank
                         │  (RRF k=60) │  fusion across providers
                         └──────┬──────┘
                                │
                    ┌───────────┴───────────┐
                    ▼                       ▼
             ┌────────────┐         ┌────────────┐
             │    JSON    │         │   Table    │
             │  (piped)   │         │ (terminal) │
             └────────────┘         └────────────┘
```

1. **Parse** -- Clap parses your query, mode, provider filter, and output preferences
2. **Route** -- Your mode determines which providers to query (or you override with `-p`)
3. **Fan out** -- `tokio::JoinSet` fires all providers in parallel with per-provider timeouts
4. **Collect** -- Once enough unique results arrive, stragglers get a 1.5s grace window, then are cancelled (reported in `metadata.providers_cancelled`; `deep` mode always waits for everyone)
5. **Fuse** -- Reciprocal rank fusion: URLs returned by multiple providers rank first (`extra.also_found_by` records consensus); ordering is deterministic, not fastest-provider-first
6. **Separate answers** -- AI-synthesized answers (Perplexity/Tavily) land in `answers[]`, so every `results[].url` is a fetchable web URL
7. **Render** -- JSON envelope when piped, colored terminal table when interactive

## Features

### 14 Search Modes

Pick a mode explicitly with `-m` (default is `general`). The `-q` column
matters: `extract`/`scrape`/`similar` take a URL and reject text queries
(exit 3). This table mirrors `search agent-info`, which is generated from the
same routing registry the engine uses.

| Mode | Use when | `-q` is | Providers used |
|------|----------|---------|----------------|
| `general` | Any web lookup not covered below (default) | query | Parallel + Brave + Serper + Exa + Jina + Linkup + Kagi + Tavily + Perplexity |
| `news` | Current events; add `-f day`/`-f week` | query | Parallel + Brave + Serper + Linkup + Kagi + Tavily + Perplexity (news endpoints) |
| `academic` | Papers/studies by topic (semantic + web) | query | Exa + Serper + Tavily + Perplexity |
| `scholar` | Google Scholar records: citations, PDFs | query | Serper + SerpApi |
| `deep` | Max coverage; waits for all providers — use `-c 30` | query | Parallel + Brave (web + LLM Context) + Serper + Exa + Linkup + Kagi + Tavily + Perplexity + xAI |
| `people` | A person, their role, LinkedIn profile | query | Exa |
| `social` | What's being said on X/Twitter | query | xAI (Grok) |
| `reddit` | Reddit threads: opinions, experience reports, troubleshooting | query | Perplexity (Search API) + Serper + Kagi + Tavily + Brave, scoped to reddit.com |
| `patents` | Prior art, patent families | query | Serper |
| `images` | Image search (check `image_url` fields) | query | Serper |
| `places` | Local businesses, maps | query | Serper |
| `extract` | Read one page as markdown, incl. JS/anti-bot | **URL** | Stealth (opt-in build) -> Jina -> Firecrawl -> Browserless; reddit.com URLs: indexed copy only (see [Reddit](#reddit)) |
| `scrape` | Alias of `extract` (identical) | **URL** | Stealth (opt-in build) -> Jina -> Firecrawl -> Browserless |
| `similar` | "More like this page" | **URL** | Exa |

### Reddit

Reddit refuses honest automated clients: www.reddit.com serves an empty JS
shell, `.json` and old.reddit answer a 403 block page, RSS ends on
2026-11-13, and the Data API closed to new apps on 2026-10-31 (unregistered
apps are cut off 2027-01-12, and everyone without a written agreement around
March 2027). Since *Reddit v. SerpApi* (S.D.N.Y. 2026-07-31), evading its bot
detection is a litigation risk. `search` therefore never requests reddit.com.
Both Reddit paths read search indexes that already hold Reddit pages:

- `-m reddit` fans the query out to Perplexity's Search API, Serper, Kagi,
  Tavily and Brave, all scoped to reddit.com. It keeps thread URLs only and
  rewrites each to one canonical URL (`https://www.reddit.com/r/<sub>/comments/<id>/`)
  so the same thread fuses across engines. `extra` carries `post_id`,
  `subreddit` and, for a comment permalink, `comment_id`. Perplexity results
  carry indexed post and comment text (up to about 4 KB). Other engines
  give a one-line snippet. `-d`/`--exclude-domain` do not apply. Exa is
  not used because it no longer indexes reddit.com.
- `-m extract` on a reddit.com or redd.it thread URL returns the thread's
  indexed copy from the Perplexity Search API, found by post ID, with source
  `reddit` and `extra.copy: "search_index"`, `extra.complete: false` and
  `extra.index_last_updated`. It is query-relevant excerpts of the post and
  comments, not the full live comment tree, and new threads appear only after
  the index catches up. When no index holds the thread, or the URL is not a
  thread (a subreddit, user page or share link), the result is an error with
  category `blocked` and code `no_sanctioned_route`, and the generic scraper
  chain does not run. `-p` without `reddit` skips this route.
- Full live threads are an opt-in: `search config set reddit.reader jina`
  (or `SEARCH_REDDIT_READER=jina`). Extract then asks Jina Reader to render the
  thread (the post and every visible comment, collapsed replies excepted,
  about 10-40 s), labelled `extra.copy: "live_render"`, and falls back to the
  index copy if Jina fails or returns a wall (`extra.live_render_error` says
  why). This is not a sanctioned route: Reddit's terms bar unauthorised
  automated access, Jina's terms make the customer responsible for the
  sources it reads, and Reddit can block Jina at any time. Every use logs a
  warning. While `reddit.reader` is unset, index copies carry
  `extra.reader_hint` offering the opt-in; `search config set reddit.reader
  index` keeps index copies and removes the hint.

Both need a Perplexity key; `-m reddit` also uses whichever of the other four
engines are configured. Terms posture: these are ordinary calls to search
APIs under their own terms. Serper resells Google results, and Perplexity is
a defendant in *Reddit v. SerpApi*, so check whether that matters for your
use.

### Rank Fusion, Not Fastest-Provider-First

Most meta-search tools return results in whatever order providers answer, so result #1 is the fastest API's opinion. `search` scores every URL across all providers (RRF, k=60): consensus ranks first, ordering is deterministic, and each result carries `extra.also_found_by` so you can see which engines agreed. The envelope tells you exactly what happened — who contributed (`provider_results`), who was cancelled (`providers_cancelled`), which filters a provider ignored (`warnings`), and whether you got a cached replay (`cached`).

### Agent-First Design

Built for Claude Code, Codex CLI, Gemini CLI, [OpenClaw](https://github.com/openclaw), and any AI agent that can shell out to a command.

```bash
# Discover capabilities programmatically
search agent-info

# Structured JSON with metadata
search "query" --json
# {
#   "status": "success",
#   "query": "...",
#   "mode": "general",
#   "results": [...],                              // rank-fused, deduped
#   "answers": [{"provider": "perplexity_fast", "text": "..."}],
#   "metadata": {
#     "elapsed_ms": 1542,
#     "result_count": 10,
#     "providers_queried": ["brave", "serper", "exa", "jina"],
#     "provider_results": {"brave": 10, "serper": 10},  // who contributed
#     "providers_cancelled": ["jina"],             // cut off after enough results
#     "warnings": []                               // e.g. filters a provider ignored
#   }
# }

# Check remaining credits (where the provider API exposes them)
search usage --json
```

**Auto-JSON:** Output is automatically JSON when piped to another program. Human-readable tables when you're in a terminal.

**Semantic exit codes:**

| Code | Meaning | Agent action |
|------|---------|-------------|
| 0 | Success | Process results |
| 1 | Transient error (API, network) | Retry might help |
| 2 | Config or auth error | Fix setup / set API key |
| 3 | Bad input | Fix arguments |
| 4 | Rate limited | Back off and retry |

### Usage Examples

```bash
# Default general web search (no mode = general; no intent guessing)
search "quantum computing advances"
search search -q "who is the CEO of Anthropic" -m people
search search -q "CRISPR research papers" -m academic

# Force a specific mode
search search -q "transformer architectures" -m academic
search search -q "Sam Altman" -m people
search search -q "AI startups 2026" -m news
search search -q "BRCA1 gene patent" -m patents

# Search X (Twitter) only
search --x "AI agents"

# Reddit threads, then one thread's post and comments
search search -q "tokio vs smol" -m reddit
search search -q https://www.reddit.com/r/rust/comments/1jc6gis/ -m extract

# URL modes take a URL, not a query
search search -q https://example.com/article -m extract
search search -q https://stripe.com -m similar

# Pick specific providers
search search -q "machine learning" -p exa
search search -q "rust programming" -p brave,serper

# Control output
search "query" --json | jq '.results[].url'
search "query" -c 20                   # 20 results
search "query" 2>/dev/null             # suppress diagnostics
```

## Providers

| Provider | What it does | Best for |
|----------|-------------|----------|
| **[Parallel](https://parallel.ai/)** | Agent-native search: objective in, LLM-ready ranked excerpts out | Grounding content for agents, deterministic per-query cost |
| **[Brave](https://brave.com/search/api/)** | Independent index (not resold Google/Bing) + LLM Context API | Independent ranking signal, news, RAG grounding |
| **[Serper](https://serper.dev/)** | Cheapest raw Google SERP + specialist endpoints | Actual Google rankings; scholar, patents, images, places |
| **[Exa](https://exa.ai/)** | Neural/semantic search, category filters | Research papers, people search, similar sites |
| **[Jina](https://jina.ai/)** | Fast URL-to-markdown, 500 RPM free tier | Reading article content, quick extraction |
| **[Linkup](https://www.linkup.so/)** | High-accuracy agent search (leads the SimpleQA benchmark) | Factual lookups where accuracy matters most |
| **[Kagi](https://help.kagi.com/kagi/api/search.html)** | Kagi's own ad-free index via Search API v1 (public preview, pay-per-use, $12/1k) | Independent, low-SEO-spam ranking signal; web + news |
| **[Firecrawl](https://firecrawl.dev/)** | JavaScript rendering, structured extraction | Dynamic pages, SPAs, data extraction |
| **[Tavily](https://tavily.com/)** | General + deep search, research-focused | Broad coverage, research queries |
| **[SerpApi](https://serpapi.com/)** | Many engines: Google, Bing, YouTube, Baidu | Multi-engine coverage; only provider with a real balance API |
| **[Perplexity](https://perplexity.ai/)** | LLM-synthesized answer with citations (Agent API, `fast` effort) | When you want an answer with sources, not raw pages |
| **Browserless** | Cloud browser for Cloudflare/JS-heavy pages | Anti-bot bypass, pages that need a real browser |
| **Stealth** | Built-in anti-bot scraper (opt-in build, `--features stealth`) | Protected pages, no API key needed |
| **[xAI](https://x.ai/)** | Only provider with native real-time X/Twitter search (Grok) | Live social signal, trending topics, account activity |

### Checking Credits

```bash
search usage --json
```

Reports remaining credits/quota for every provider whose API exposes it:
**SerpApi** (`account.json`), **Firecrawl** (credit-usage endpoint),
**Tavily** (usage endpoint), **Linkup** (credits balance endpoint), **Serper**
(account endpoint), **Jina** (token wallet, read from Jina's API-dashboard
backend, which is not part of its public docs), **xAI**
(management API — set `XAI_MANAGEMENT_API_KEY` + `XAI_TEAM_ID`), and **Brave**
(rate-limit headers; the check consumes one metered request). The rest are dashboard-only and
reported as `supported: false` (Kagi API v1 exposes no balance at all). Purely informational: the
CLI never disables or deprioritizes a provider because its balance is low —
if a provider is out of credits, the failed call shows up as a
`billing_quota` entry in `metadata.provider_failures` and you decide what to
top up.

### Monthly Billing

```bash
search billing           # table: providers x last 6 months, newest left
search billing --json    # same data, every cell tagged with its source
```

One row per provider, one column per UTC calendar month (last six, newest
first). Values stay in each provider's own unit; nothing is converted to
dollars. Each cell says where its number came from:

| Cell | Source | Providers |
|---|---|---|
| `197M` | provider usage history (`api`) | **Jina** (tokens), **Firecrawl** (credits), **xAI** (USD; needs `XAI_MANAGEMENT_API_KEY` + `XAI_TEAM_ID`) |
| `7,030*` | current billing cycle only (`api_current_period`), current month column | **Tavily** (credits), **SerpApi** (searches) |
| `~$0.18` | estimate from local search logs (`estimate`, USD, `search stats` price table) | everyone else, and earlier months of Tavily/SerpApi |
| `-` | no data (`none`): month predates the oldest local search log | |

If a provider's usage API fails, its row falls back to estimates and the
error is listed under the table (`error` in JSON). Exa's usage history needs
a separate team service key, so Exa stays on estimates.

### Health Check

```bash
search doctor --json
```

Test-fires every configured provider with a minimal query and reports
`ok`/`fail`, latency, and failure category — the preflight to run before a
fleet trusts the tool. `config check` tells you a key exists; `doctor` tells
you it works. Each check is one billed (minimal) request. Exit 0 if all
healthy, 1 otherwise.

### Usage Analytics

```bash
search stats --json          # last 30 days
search stats --days 7        # custom window
search stats --prune 90      # delete logs older than 90 days
```

Computed entirely from the local search logs — nothing leaves the machine.
Reports search volume by mode, per-provider call counts / failures /
cancellations, estimated spend (from a static price table), **measured**
balance burn (deltas between `search usage` snapshots), cache-hit rate,
repeated queries, and a read-through table — which providers' results agents
actually extract afterwards, the closest local proxy for result usefulness.
Set `SEARCH_LOG=off` to disable logging entirely.

## Configuration

Config file lives at `~/.config/search/config.toml` (Linux) or `~/Library/Application Support/search/config.toml` (macOS).

```bash
search config show          # View current config (keys masked)
search config check         # Which providers have a key set
search config set K V       # Set a value
echo "$KEY" | search config set keys.brave -   # read secret from stdin
```

The config file holds API keys, so it's created `0600` (owner-only) and any
older world-readable file is tightened on load. Set secrets with `-` as the
value to read from stdin and keep keys out of shell history and `ps`.

Environment variables override the config file, so a freshly-exported or
CI-injected key always wins over stale local config. Two accepted forms:

```bash
# Standard per-provider variables (recommended):
export BRAVE_API_KEY=your-key
export SERPER_API_KEY=your-key
export EXA_API_KEY=your-key
# ...also PERPLEXITY_API_KEY, JINA_API_KEY, LINKUP_API_KEY, KAGI_API_KEY, FIRECRAWL_API_KEY,
#        TAVILY_API_KEY, SERPAPI_API_KEY, BROWSERLESS_API_KEY, XAI_API_KEY,
#        PARALLEL_API_KEY

# Or the SEARCH_KEYS_ prefixed form:
export SEARCH_KEYS_BRAVE=your-key
```

Precedence (highest first): `<PROVIDER>_API_KEY` env → `SEARCH_KEYS_*` env → config file.

xAI's model is configurable because xAI retires model slugs on a schedule.
The default is `grok-4.3`; override it with `search config set xai.model <id>`
or `SEARCH_XAI_MODEL`. In `social` mode, `-d` restricts results to the given X
handles and `--exclude-domain` drops them (without `@`, at most 20 each). xAI
accepts only one of the two lists per request, so `-d` wins if both are given.

## Signing in to metasearch.sh

[metasearch.sh](https://metasearch.sh) is the hosted sibling: one prepaid
balance, one key, per-call metering. `search login` does the OAuth device flow
— it prints a short code, opens your browser, and stores the key it gets back.

**What this does today:** it stores the credential. Searches still run through
your own provider keys. Routing queries through metasearch.sh — so one machine
needs one key instead of thirteen — is the next release.

```bash
search login              # code + browser approval, key saved to config
search login --no-browser # headless: prints the URL to open elsewhere
search whoami             # which account, what balance, which key
search logout --revoke    # clear locally and kill the key server-side
```

Repeat logins from the same machine reuse that machine's key rather than
minting a new one, and every key shows up in the metasearch.sh dashboard
labelled with the hostname it was issued to.

Precedence (highest first): `METASEARCH_TOKEN` env → `SEARCH_METASEARCH_TOKEN`
env → config file. `METASEARCH_URL` overrides the API host for self-hosted or
local instances.

## Updating

```bash
search update             # Self-update from GitHub releases
search update --check     # Check without installing
```

## Building from Source

```bash
git clone https://github.com/alepar/search-cli
cd search-cli
cargo build --release
# Binary at target/release/search
```

**Stealth scraper (opt-in):** the local `stealth` rung of the extract chain
impersonates a browser's TLS fingerprint through BoringSSL. It is not built by
default and not included in release binaries; without it the extract chain
starts at Jina. To build it you need Rust 1.98+, cmake, clang and a C++
compiler:

```bash
cargo install --git https://github.com/alepar/search-cli --features stealth
```

Prebuilt binaries ship for macOS (both architectures) and Linux x86_64; every
release includes a `SHA256SUMS` file, and both `install.sh` and self-update
verify the checksum before installing.

## Contributing

Contributions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## License

[MIT](LICENSE)

---

<div align="center">

Built by [Boris Djordjevic](https://github.com/longevityboris) at [Paperfoot AI](https://paperfoot.com)

<br />

**If this is useful to you:**

[![Star this repo](https://img.shields.io/github/stars/paperfoot/search-cli?style=for-the-badge&logo=github&label=%E2%AD%90%20Star%20this%20repo&color=yellow)](https://github.com/paperfoot/search-cli/stargazers)
&nbsp;&nbsp;
[![Follow @longevityboris](https://img.shields.io/badge/Follow_%40longevityboris-000000?style=for-the-badge&logo=x&logoColor=white)](https://x.com/longevityboris)

</div>
