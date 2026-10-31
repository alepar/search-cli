# Academic publishers vs. agentic retrieval, and the open scholarly infrastructure to use instead (as of 2026-10-31)

Method note: besides web sources, this researcher ran live checks on 2026-10-31 from a residential Linux host with `curl` and a Firefox User-Agent. The checks fetched robots.txt files, sent requests to article pages and captured response headers, and called the open APIs keylessly. Findings from these checks are cited as "[live probe 2026-10-31]" with the URL that was probed. A probe is one request from one IP, so a challenge or block is a strong signal but not a guarantee of behaviour for every client.

## 1. Restrictive publishers/platforms: bot protection, robots.txt, paywalls, TDM terms

### Takeaway
Every major commercial platform tested serves a bot challenge to a plain HTTP client. Cloudflare protects ScienceDirect, Wiley, ACM, T&F, SSRN, ResearchGate and Academia.edu; AWS WAF protects IEEE; Springer and JSTOR serve a "Client Challenge" interstitial. Most robots.txt files explicitly disallow AI agents, and several now name user-initiated fetchers (Claude-User, ChatGPT-User, Perplexity-User, even `claude-code`). Scraping publisher HTML is therefore neither technically workable nor sanctioned. The only legitimate full-text routes are the publishers' TDM APIs, and those are gated on an institutional subscription, limited to non-commercial use and restricted in what output may be shown.

### Cited Findings
**Bot protection (live probes, Firefox UA, 2026-10-31)**
- ScienceDirect article page: HTTP 403, `server: cloudflare`, `cf-mitigated: challenge` — [live probe](https://www.sciencedirect.com/science/article/pii/S0140673620301835)
- Wiley Online Library: 403, Cloudflare `cf-mitigated: challenge` — [live probe](https://onlinelibrary.wiley.com/doi/10.1002/asi.24445)
- ACM DL: 403, Cloudflare challenge — [live probe](https://dl.acm.org/doi/10.1145/3442188.3445922)
- Taylor & Francis Online: 403, Cloudflare challenge. Its `/.well-known/tdmrep.json` (which robots.txt explicitly allows) also returned a Cloudflare "Just a moment..." page, as did Wiley's. Under the probe, the TDMRep opt-out file sits behind the bot wall it is meant to explain — [live probe](https://www.tandfonline.com/.well-known/tdmrep.json)
- SSRN (papers.ssrn.com, an Elsevier property): 403, Cloudflare challenge — [live probe](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=3000000)
- ResearchGate publication page: 403, Cloudflare (`__cf_bm` cookie) — [live probe](https://www.researchgate.net/publication/220566236)
- Academia.edu homepage: 403, Cloudflare challenge — [live probe](https://www.academia.edu/)
- IEEE Xplore document page: HTTP 202, `server: CloudFront`, `x-amzn-waf-action: challenge` (AWS WAF JS challenge) — [live probe](https://ieeexplore.ieee.org/document/9000000)
- Springer Link article and JSTOR stable page: HTTP 200, but the body is a `<title>Client Challenge` interstitial rather than the article. Springer's robots.txt itself returned this challenge HTML — [live probe Springer](https://link.springer.com/article/10.1007/s11192-019-03217-6); [live probe JSTOR](https://www.jstor.org/stable/1885099)
- Google Scholar `/scholar?q=` returned 200 to a single request (`server: scholar`). Scholar's robots.txt disallows `/scholar` and `/search` for all agents — [live probe](https://scholar.google.com/robots.txt)
- No Akamai or PerimeterX/HUMAN signatures (`_abck`, `ak_bmsc`, `_px*`) or DataDome cookies were seen on any of the tested hosts. Cloudflare dominates, and IEEE uses AWS WAF. Note that the probe looked at headers only — [live probes above]

**robots.txt (fetched 2026-10-31)**
- ScienceDirect (header "Last updated: October 2026"): `User-agent: * / Disallow: /` (default deny), with an allowlist for search engines. A comment says "OAI-SearchBot and Claude-SearchBot are explicitly allowed to enable AI search citations. All others remain blocked pending review." It explicitly disallows ChatGPT-User, Claude-User, **`claude-code`** ("Anthropic (Claude Code CLI)"), MistralAI-User, PerplexityBot, Perplexity-User, Meta-ExternalFetcher, Google-Agent (Project Mariner) and Google-NotebookLM, plus GPTBot, ClaudeBot, CCBot, Google-Extended, Bytespider, Diffbot and AI2Bot — [robots.txt](https://www.sciencedirect.com/robots.txt)
- IEEE Xplore: `Disallow: /` for GPTBot, Google-Extended, ClaudeBot, Claude-User, Google-NotebookLM, Perplexity-User, PerplexityBot, MistralAI-User/Index, meta-externalagent, Amazonbot, Applebot-Extended, CCBot, DuckAssistBot, YouBot, Diffbot, Bytespider, **ResearchGateBot**, quillbot and others. All agents are disallowed from `/rest` and `/ielx*` (the PDF paths) — [robots.txt](https://ieeexplore.ieee.org/robots.txt)
- Nature.com (Springer Nature): `Disallow: /` for anthropic-ai, Claude-Web, ClaudeBot, GPTBot, ChatGPT-User, CCBot, Google-Extended, PerplexityBot, Applebot-Extended, Amazonbot, Bytespider, cohere-ai, meta-externalagent, Diffbot and omgili — [robots.txt](https://www.nature.com/robots.txt)
- Wiley Online Library: `Disallow: /` for GPTBot and Google-Extended only. For everyone, `/doi/am-pdf/` and `/search` are disallowed, `Crawl-delay: 1` applies, and `/.well-known/tdmrep.json` is explicitly allowed — [robots.txt](https://onlinelibrary.wiley.com/robots.txt)
- ACM DL: `Disallow: /` for GPTBot, ChatGPT-User, CCBot and Google-Extended. `/action/`, `/search/` and `/author/` are disallowed for all, with `Crawl-delay: 1` — [robots.txt](https://dl.acm.org/robots.txt)
- JSTOR: `Disallow: /` for ClaudeBot, CCBot, GPTBot and Google-Extended. For all agents, `/stable/full`, `/stable/view`, `/api` and `/citation` are disallowed — [robots.txt](https://www.jstor.org/robots.txt)
- SSRN: `Disallow: /` for GPTBot, ChatGPT-User and Google-Extended. All other agents get `Crawl-Delay: 5` — [robots.txt](https://papers.ssrn.com/robots.txt)
- Taylor & Francis: `Disallow: /` for GPTBot. `/doi/epdf/*` and `/doi/epub/*` are disallowed for all, and tdmrep.json is allowed — [robots.txt](https://www.tandfonline.com/robots.txt)
- Academia.edu robots.txt (about 59 KB) has explicit groups for GPTBot, ChatGPT-User, OAI-SearchBot, Claude-SearchBot, Claude-User, PerplexityBot, MistralAI-User, Google-Extended and Applebot. The Claude-User group seen disallows only a few paths (`/ads.txt`, `/upgrade-browser`, `/cdn-cgi/`), so its robots.txt is less restrictive than its Cloudflare wall — [robots.txt](https://www.academia.edu/robots.txt)
- ResearchGate: robots.txt is permissive (`User-agent: * Allow: /`, with a few disallows), yet article pages are Cloudflare-challenged. Enforcement is at the WAF, not in robots.txt — [robots.txt](https://www.researchgate.net/robots.txt)
- bioRxiv (an *open* preprint server) disallows ClaudeBot, GPTBot, ChatGPT-User, PerplexityBot, CCBot and Bytespider, with `Crawl-delay: 7`. Scrape-style access to the site itself is unwelcome; use api.biorxiv.org instead — [robots.txt](https://www.biorxiv.org/robots.txt)

**TDM policies and TDM APIs**
- Elsevier: researchers at subscribing academic institutions may text-mine subscribed ScienceDirect full text through the Elsevier APIs for non-commercial purposes. Each researcher self-registers for a personal API key on the developer portal. Others must contact permissions — [Elsevier TDM policy](https://www.elsevier.com/about/policies-and-standards/text-and-data-mining)
- Elsevier TDM licence restrictions: output may not include snippets longer than 200 characters from individual full-text articles or chapters, "robots, spiders or other automated downloading programs" against Elsevier sites are prohibited, and keys may not be shared — [Elsevier TDM license](https://www.elsevier.com/about/policies-and-standards/text-and-data-mining/license); [Elsevier TDM FAQ](https://www.elsevier.com/about/policies/text-and-data-mining/text-and-data-mining-faq)
- Elsevier's FAQ says the service supports "non-commercial text and data mining for academic research purposes". Corporate users are directed to account managers — [Elsevier TDM FAQ](https://www.elsevier.com/about/policies/text-and-data-mining/text-and-data-mining-faq)
- Springer Nature: TDM rights for subscribed content come through the subscribing institution and cover non-commercial research only. Researchers can request a TDM API key, which gives "a higher bandwidth for content requests (150 requests per minute)". Commercial TDM is offered for a fee. Non-subscribers can use the Open Access full-text API — [Springer Nature TDM](https://www.springernature.com/text-and-data-mining)
- The Springer Nature Open Access API rejects keyless calls: `"API key is invalid or missing"` — [live probe](https://api.springernature.com/openaccess/json?q=doi:10.1186/s13059-019-1758-4)
- Wiley: a click-through TDM licence issues a Client API Token. Academic subscribers obtain it with their ORCID iD, and it gives access only to content their institution subscribes to. Non-commercial TDM costs nothing extra. Contact tdm@wiley.com — [W&M library TDM guide](https://guides.libraries.wm.edu/tdm); [UW TDM API guide](https://guides.lib.uw.edu/research/text-mining/data/api) (secondary sources)
- IEEE Xplore has a separate API host. A keyless call to `ieeexploreapi.ieee.org/api/v1/search/articles` returned HTTP 403, so a key is required — [live probe](https://ieeexploreapi.ieee.org/api/v1/search/articles?querytext=transformer)

### Inferences
- An agentic CLI that sends a browser UA to these hosts will mostly receive challenge pages. Solving them, whether with headless browsers or challenge-solver services, would circumvent technical measures and violate robots.txt directives that now name user-initiated AI fetchers. Elsevier also names the Claude Code CLI UA specifically. The CLI should recognise these domains and route to the metadata/OA pipeline in Section 2 instead of fetching them.
- The TDM APIs are built for an individual institutional researcher doing non-commercial work. They are not built for a general-purpose CLI shipped to arbitrary users. At most, the CLI could support "bring your own Elsevier/Springer/Wiley key" as opt-in configuration, and its terms would still cap displayed snippets (Elsevier: 200 characters).
- A useful heuristic: a 403 with `cf-mitigated: challenge`, a 202 with `x-amzn-waf-action: challenge`, or `<title>Client Challenge` / `Just a moment...` should be classified as "blocked by publisher", not retried.

### Gaps
- ACM DL now offers open access for much of its corpus (ACM's OA transition) and has its own TDM terms. Neither was verified in this pass.
- JSTOR's TDM offering (Constellate, now folded into JSTOR) and its 2026 status were not verified.
- Current IEEE Xplore API quotas and terms, including any AI clause, were not fetched.
- No primary source was found for any 2026 change to Elsevier's TDM terms that is specific to AI or LLMs. A third-party claim about Scopus AI licensing was unsourced and has been excluded.
- No public TDM or API exists for ResearchGate or Academia.edu. They are not legitimate sources for automated retrieval. Their terms of service were not fetched.

## 2. Best legitimate pipeline for a DOI or publisher URL

### Takeaway
Resolve to a DOI first. Get metadata and abstract from Crossref, OpenAlex, Semantic Scholar or Europe PMC. Find open full text through Unpaywall `best_oa_location` and Semantic Scholar `openAccessPdf`. Prefer repository or OA copies on arXiv (abs/HTML/PDF), PMC (via the BioC API or Europe PMC `fullTextXML`) or bioRxiv over the publisher landing page. All of these steps worked keylessly in live tests on 2026-10-31.

### Cited Findings
- Crossref `GET /works/{doi}?mailto=...` returned title, an abstract (`abstract` present for the AlphaFold paper), `link[]` full-text URLs (nature.com PDF) and a licence URL (CC BY 4.0) — [live probe](https://api.crossref.org/works/10.1038/s41586-021-03819-2)
- Unpaywall `GET /v2/{doi}?email=...` returned `is_oa: true`, `oa_status: hybrid`, and `best_oa_location.url_for_pdf` set to the nature.com PDF. Without `email` it returns HTTP 422: "Email address required in API call" — [live probe](https://api.unpaywall.org/v2/10.1038/s41586-021-03819-2)
- Semantic Scholar `GET /graph/v1/paper/DOI:{doi}?fields=title,openAccessPdf,externalIds,abstract` returned `externalIds` (PMC, PubMed, DOI, CorpusId) and `openAccessPdf` {url, status: HYBRID, license: CCBY}. Its disclaimer pointed to the PMC copy — [live probe](https://api.semanticscholar.org/graph/v1/paper/DOI:10.1038/s41586-021-03819-2?fields=title,openAccessPdf,externalIds,abstract)
- Europe PMC `search?query=DOI:{doi}` returned the PMID, the PMCID and `fullTextIdList`. `GET /{PMCID}/fullTextXML` returned the full JATS XML (177 KB) — [live probe search](https://www.ebi.ac.uk/europepmc/webservices/rest/search?query=DOI:10.1038/s41586-021-03819-2&format=json&resultType=lite); [live probe fullTextXML](https://www.ebi.ac.uk/europepmc/webservices/rest/PMC8371605/fullTextXML)
- The NCBI BioC API (`/research/bionlp/RESTful/pmcoa.cgi/BioC_json/{PMCID}/unicode`) returned structured full text in JSON with a licence `infons` field — [live probe](https://www.ncbi.nlm.nih.gov/research/bionlp/RESTful/pmcoa.cgi/BioC_json/PMC8371605/unicode)
- The legacy PMC OA web service URL `www.ncbi.nlm.nih.gov/pmc/utils/oa/oa.fcgi` returned 404. PMC has moved to pmc.ncbi.nlm.nih.gov, and the OA service endpoint has apparently moved or been retired — [live probe](https://www.ncbi.nlm.nih.gov/pmc/utils/oa/oa.fcgi?id=PMC8371605)
- The arXiv API (`export.arxiv.org/api/query?id_list=1706.03762`) returned an Atom entry with title and abstract. arXiv HTML renderings exist (`arxiv.org/html/1706.03762` returned 200) — [live probe](https://export.arxiv.org/api/query?id_list=1706.03762)
- The bioRxiv/medRxiv API `api.biorxiv.org/details/biorxiv/{doi}` returned metadata keylessly — [live probe](https://api.biorxiv.org/details/biorxiv/10.1101/2020.03.22.002386)
- OpenAlex `GET /works/doi:{doi}` worked keylessly, returning ids, title and dates, and its headers carried cost accounting — [live probe](https://api.openalex.org/works/doi:10.1038/s41586-021-03819-2)
- DOAJ `api/search/articles/doi:{doi}` returned the article record keylessly — [live probe](https://doaj.org/api/search/articles/doi:10.1186%2Fs13059-019-1758-4)

### Inferences
Recommended pipeline for a single item:
1. **Normalise input.** From a publisher URL, extract the DOI (Elsevier PII and IEEE document numbers do not contain one) using page-free methods: patterns in the URL, then a Crossref `query.bibliographic` search on the title if needed. Never fetch the blocked landing page to read `citation_doi` meta tags.
2. **Get metadata and abstract** from Crossref (abstracts only when the publisher deposits them; many Elsevier records lack them), then OpenAlex (`abstract_inverted_index`), then Semantic Scholar `abstract`, then Europe PMC (biomedical).
3. **Look for an OA location**: Unpaywall `best_oa_location` (`url_for_pdf`, `url_for_landing_page`, `host_type` repository or publisher, licence), then S2 `openAccessPdf`, then OpenAlex `best_oa_location` / `locations[]`.
4. **Prefer a structured full-text version** over a publisher PDF. If a PMCID exists, use Europe PMC `fullTextXML` or BioC JSON. If an arXiv ID exists, use arXiv HTML or PDF (respecting 1 request per 3 seconds). For bioRxiv or medRxiv, use the API to get the version, then the PDF. Note that bioRxiv's robots.txt bars AI-named bots, which bears on what UA to send.
5. **If the OA location is on a Cloudflare-walled publisher host** (e.g., Wiley hybrid OA PDF), expect a challenge. Fall back to a repository copy (`locations[]` with `host_type=repository`, CORE), or return metadata, abstract and the link.
6. **Otherwise return metadata, abstract and the DOI link.** Do not attempt the paywall.

### Gaps
- No probe tested whether OA PDFs on publisher hosts (e.g., nature.com PDF links from Unpaywall) pass the bot wall for a non-browser client.
- The new PMC OA bulk/service endpoint (after the PMC site migration) was not identified.

## 3. Quotas, auth and terms of each open API (2026)

### Takeaway
Most open APIs still work without a key, but the trend in 2025–2026 is toward mandatory identification and metering. OpenAlex now runs a dollar-denominated credit budget: about $0.10/day keyless and $1/day with a free key. Crossref tightened its pools on 2025-12-01. Semantic Scholar's shared keyless pool was exhausted (429) at probe time. CORE's keyless limit is 10. A CLI should ship with "bring your own key/email" support for OpenAlex, S2, NCBI, CORE and Unpaywall.

### Cited Findings
**OpenAlex**
- Docs ("Last updated October 15, 2026"): an API key is optional for basic queries. A free key raises the daily budget 10× and is required for cursor paging beyond the first page. Keys work only after email verification. Rate limits are 10 req/s with no key, 30 req/s with a free key or prepaid usage, and 50 req/s on an annual plan. Semantic search is limited to 1 req/s. Queries with more than 5 boolean operators get 1 req/s keyless, 5 with a key or prepaid usage, and 25 on an annual plan. `per_page` is at most 100, basic paging stops at 10,000 results, and OR filters take at most 100 values. An "agent-issued key" gets a $1/day budget after activation — [OpenAlex rate limits & auth](https://help.openalex.org/how-to-use-the-api/rate-limits-and-authentication)
- Pricing page: "Every account gets $1 per day free". Prepaid usage starts at $1, annual subscriptions are available, and PDF sync is offered on Member+/Partner plans — [OpenAlex pricing](https://openalex.org/pricing)
- Live keyless response headers: `x-ratelimit-limit-usd: 0.1`, `x-ratelimit-remaining-usd: 0.1`, `x-ratelimit-cost-usd: 0` for a singleton DOI lookup, and `x-ratelimit-limit: 1000`. The headers also expose credits, prepaid balance and `Retry-After`. This implies a keyless budget of $0.10/day and that singleton lookups are free — [live probe](https://api.openalex.org/works/doi:10.1038/s41586-021-03819-2)

**Crossref REST API**
- New limits took effect 2025-12-01, the first change since the API launched in 2013. Public pool (no mailto): single-record 5, list queries 1, concurrency 1. Polite pool (`mailto=`): single-record 10 with concurrency 3, list queries 3 with concurrency 3. Metadata Plus (paid) is unchanged and recommended for production — [Crossref blog](https://crossref.org/blog/announcing-changes-to-rest-api-rate-limits/); [community post](https://community.crossref.org/t/announcing-changes-to-rest-api-rate-limits-crossref/14726)
- Confirmed live: `x-api-pool: polite-single`, `x-rate-limit-limit: 10`, `x-rate-limit-interval: 1s`, `x-concurrency-limit: 3` — [live probe](https://api.crossref.org/works/10.1038/s41586-021-03819-2?mailto=test@example.org)

**Semantic Scholar Graph API**
- Unauthenticated access shares "1000 requests per second shared among all unauthenticated users" and may be throttled. The introductory key limit is "1 RPS on all endpoints". Keys are requested by form and delivered by email — [S2 API product page](https://www.semanticscholar.org/product/api)
- Live keyless `/paper/search` returned HTTP 429 (shared pool exhausted), while a DOI lookup moments earlier succeeded. Keyless use is unreliable — [live probe](https://api.semanticscholar.org/graph/v1/paper/search?query=transformer&limit=1)
- The licence (last updated 2023-05-17) is limited, non-transferable and terminable at will. Repackaging or redistributing the API is prohibited. Data is under the attached licences, CC BY-NC or ODC-BY. Attribution "Semantic Scholar" is required, and rate-limit circumvention is grounds for suspension. There is no explicit AI/LLM clause — [S2 API License](https://www.semanticscholar.org/product/api/license)

**arXiv**
- API, OAI-PMH and RSS: no more than one request every 3 seconds, a single connection at a time, with limits applying across all of a user's machines. Metadata is CC0. E-prints are copyright of their authors or publishers; you may retrieve and store them for personal or research use but not re-serve them without permission or a permitting licence. Do not imply arXiv endorsement. There is no explicit AI/LLM clause — [arXiv API Terms of Use](https://info.arxiv.org/help/api/tou.html)
- arxiv.org robots.txt: `Crawl-delay: 15` for `*`. `/abs`, `/pdf`, `/html` and `/list` are allowed. `/api`, `/search`, `/e-print` and `/src` are disallowed on arxiv.org. Use the export.arxiv.org API host for programmatic queries — [robots.txt](https://arxiv.org/robots.txt)

**NCBI E-utilities / PMC**
- Live keyless response: `x-ratelimit-limit: 3` (3 req/s without an API key) — [live probe](https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi?db=pmc&term=10.1038/s41586-021-03819-2%5Bdoi%5D&retmode=json)

**CORE API v3**
- Live keyless response: HTTP 429, `x-ratelimit-limit: 10`, `x-ratelimit-remaining: 0`, and a retry-after about 10 minutes later. The keyless allowance is tiny or shared; register for a key — [live probe](https://api.core.ac.uk/v3/search/works?q=doi:%2210.1038/s41586-021-03819-2%22)

**Unpaywall**
- An `email` parameter is mandatory; calls without it get HTTP 422 — [live probe](https://api.unpaywall.org/v2/10.1038/s41586-021-03819-2)

**Europe PMC, bioRxiv, DOAJ**
- All answered keylessly with no rate-limit headers observed — [live probes above]

### Inferences
- Recommended CLI config: `OPENALEX_API_KEY`, `S2_API_KEY`, `NCBI_API_KEY`, `CORE_API_KEY`, plus a contact email used for both Crossref `mailto` and Unpaywall `email`. Honour `Retry-After` and the `x-rate-limit-*` / `x-ratelimit-*` headers. Throttle arXiv to 1 request per 3 seconds globally.
- OpenAlex keyless use ($0.10/day) is enough for occasional singleton lookups, which cost $0 in the probe, but not for list or search workloads. The tool should surface the budget-remaining headers.
- S2's CC BY-NC data licence matters if the CLI is used commercially.

### Gaps
- OpenAlex per-endpoint costs (list, search, content download) were not obtained, because the "Example costs" page was not fetched. The date OpenAlex introduced keys and USD billing (believed 2025) was not confirmed from a primary changelog.
- The NCBI 10 req/s with-key limit and the E-utilities usage policy were not confirmed from the docs page in this pass. Only the keyless 3 req/s was observed live.
- Unpaywall's daily cap (historically 100k/day) and Unpaywall's 2025–2026 changes (it is now operated by OurResearch alongside OpenAlex) were not verified.
- CORE, Europe PMC and bioRxiv published terms (including any AI clauses) were not fetched.
- No open API reviewed contains an explicit AI/LLM-use clause, apart from absence-of-clause findings for arXiv and S2.

## 4. AI crawler blocking, lawsuits and AI licensing deals

### Takeaway
Publishers block AI crawlers on their own sites while selling their corpora to AI companies. Wiley and Taylor & Francis disclosed AI licensing revenue from 2024 onward. Elsevier is a co-plaintiff in a May 2026 copyright suit against Meta over Llama training.

### Cited Findings
- On 2026-05-05, Elsevier, Cengage, Hachette, Macmillan and McGraw Hill (joined by author Scott Turow) filed a proposed class action against Meta in Manhattan federal court. They allege that Meta trained Llama on their books and journal articles obtained from pirate sources. Meta says training is fair use — [The Star/Reuters](https://www.thestar.com.my/tech/tech-news/2026/05/05/major-publishers-sue-meta-for-copyright-infringement-over-ai-training); [Claims Journal](https://www.claimsjournal.com/news/national/2026/05/06/337385.htm); [Research Information](https://www.researchinformation.info/?p=8744)
- Wiley has disclosed AI rights revenue. The Bookseller reported $44m from AI rights deals and that authors get no opt-out. A CASRAI piece reports $49m in FY2026 and more than $110m lifetime, and flags that figure as not matching Wiley's investor release. **The sources conflict**, so check Wiley's investor filings — [The Bookseller](https://thebookseller.com/news/wiley-set-to-earn-44m-from-ai-rights-deals-confirms-no-opt-out-for-authors); [CASRAI](https://www.casrai.org/news/scholarly-publisher-ai-licensing-deals-2026)
- Wiley, Cambridge UP and Oxford UP confirmed AI partnerships — [The Bookseller](https://www.thebookseller.com/news/wiley-cambridge-university-press-and-oxford-university-press-confirm-ai-partnerships)
- Taylor & Francis signed a reported $10m deal with Microsoft (2024) for training access. Authors objected that they were not notified and had no opt-out — [Interesting Engineering](https://interestingengineering.com/culture/publishers-sell-data-ai-earn-millions); [Authors Guild](https://authorsguild.org/news/ag-demands-prior-consent-for-ai-use-of-academic-and-news-content/)
- A Springer Nature–Google deal (2024, reported at $23m one-time) appears only in CASRAI and is **uncorroborated** — [CASRAI](https://www.casrai.org/news/scholarly-publisher-ai-licensing-deals-2026)
- HKUST Library maintains a tracker of scholarly AI-licensing deals — [HKUST Library](https://library.hkust.edu.hk/news-events/news/tracking-licensing-scholarly-content-ai-what-researchers-need-know)
- Robots.txt blocks naming AI agents, as summarised in Section 1: ScienceDirect (default-deny, including `claude-code`), IEEE (about 25 AI agents including Claude-User), Nature, JSTOR, ACM, SSRN, Wiley, T&F and bioRxiv — [robots.txt files linked in Section 1]

### Inferences
- This is a licence-and-block regime: publishers sell bulk corpora to model vendors under contract and block unlicensed and user-initiated agents at the edge. For a CLI, this means the publisher HTML route will not open up, so the open metadata/OA layer is the durable path.
- ScienceDirect's split, which allows Claude-SearchBot and OAI-SearchBot (index for citation) but disallows Claude-User and claude-code (live fetch), signals that publishers want to be cited in AI search without being read by agents.

### Gaps
- No primary Wiley investor-release figure for FY2026 AI revenue was retrieved.
- No information was found on whether Elsevier, Springer Nature or IEEE licensed content to specific LLM vendors in 2025–2026, beyond the items above.
- No publisher lawsuit against an *agentic retrieval* tool (as opposed to training) was found.

## 5. Google Scholar: any sanctioned access?

### Takeaway
No. Google Scholar has no public API. Its robots.txt disallows `/scholar` and `/search` for every agent, and Google is actively litigating against SerpApi, the best-known SERP scraper that includes Scholar endpoints. A legitimate CLI should replace Scholar with OpenAlex, Semantic Scholar or Crossref search.

### Cited Findings
- Scholar robots.txt: `User-agent: *` disallows `/search`, `/scholar` and `/citations?` (with some profile and metrics pages allowed) — [robots.txt](https://scholar.google.com/robots.txt)
- Google sued SerpApi under the DMCA §1201 anti-circumvention provision; reports place the filing in December 2025. On 2026-07-20, Judge Yvonne Gonzalez Rogers (N.D. Cal.) dismissed the §1201 claims, reasoning that the search results at issue contain no copyrighted material. Reports differ on whether leave to amend was granted. The suit has no CFAA claim — [Search Engine Land](https://searchengineland.com/google-loses-key-dmca-claims-against-serpapi-in-scraping-lawsuit-483185); [The Next Web](https://thenextweb.com/news/google-serpapi-dmca-scraping-lawsuit-dismissed); [MediaPost](https://www.mediapost.com/publications/article/416687/judge-dismisses-google-complaint-against-serpapi-o.html)
- Google later renewed the fight with an amended complaint (MediaPost headline "Google Renews Battle With SerpApi Over Scraping"; the article itself returned 403 to fetch). Reddit separately has a DMCA suit against SerpApi — [MediaPost](https://www.mediapost.com/publications/article/417185/google-renews-battle-with-serpapi-over-scraping.html); [Search Engine Journal](https://searchenginejournal.com/court-dismisses-googles-dmca-claims-against-serpapi/583033)
- Commentators stress that this is one district-court ruling on one statute and does not make scraping legal in general — [ScrapeBadger analysis](https://scrapebadger.com/blog/google-sued-a-scraper-under-copyright-law-and-lost-heres-what-the-serpapi-ruling-actually-says); [Parallel](https://parallel.ai/articles/is-scraping-google-legal.md)

### Inferences
- Even after the July 2026 ruling, relying on SerpApi or similar Scholar scrapers carries litigation, ToS and availability risk for a CLI. Scholar's robots.txt alone excludes it under a robots-respecting policy.
- Functional replacements: OpenAlex (search plus citation graph), Semantic Scholar (search, citations, recommendations) and Crossref (bibliographic matching).

### Gaps
- The exact filing date of the original complaint and the content of the amended complaint were not verified from primary court records.
- No information was found on whether Google offers any Scholar data-sharing programme in 2026. None is known, but absence was not formally confirmed.
