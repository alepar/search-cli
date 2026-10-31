//! Papers by identifier instead of through publisher pages, which sit behind
//! bot walls and robots rules that name AI agents. A DOI (from doi.org or a
//! publisher URL), arXiv id, PubMed/PMC id or Elsevier PII resolves through
//! OpenAlex (Crossref and Unpaywall when OpenAlex lacks the work); full text
//! comes from Europe PMC, arXiv's HTML rendering, or an open-access landing
//! page, and otherwise the result is metadata, abstract and OA links.
//!
//! `settings.email` is sent as the polite-pool contact (OpenAlex/Crossref
//! `mailto`) and is required by Unpaywall, which is skipped without it.

use super::{get, page, Route};
use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::SearchResult;
use crate::walls;
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;
use url::Url;

const NAME: &str = "scholarly";
const OPENALEX: &str = "https://api.openalex.org/works";
const CROSSREF: &str = "https://api.crossref.org/works";

pub struct Scholarly {
    ctx: Arc<AppContext>,
}

impl Scholarly {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Id {
    Doi(String),
    Arxiv(String),
    Pmid(String),
    Pmcid(String),
    Pii(String),
}

/// Pull a scholarly identifier out of a URL. Publisher URLs mostly embed the
/// DOI in the path ("/doi/10.1145/...", "/article/10.1007/..."); the rest
/// have their own id schemes.
fn identify(url: &Url) -> Option<Id> {
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.trim_start_matches("www.");
    let path = url.path();
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match host {
        "arxiv.org" | "export.arxiv.org" => {
            let id = match segs.as_slice() {
                ["abs" | "pdf" | "html", rest @ ..] if !rest.is_empty() => rest.join("/"),
                _ => return None,
            };
            let id = id.trim_end_matches(".pdf");
            // Versions ("v3") are dropped: OpenAlex and arXiv HTML want the base id.
            let base = match id.rfind('v') {
                Some(i)
                    if i > 0
                        && id[i + 1..].chars().all(|c| c.is_ascii_digit())
                        && i + 1 < id.len() =>
                {
                    &id[..i]
                }
                _ => id,
            };
            return Some(Id::Arxiv(base.to_string()));
        }
        "pubmed.ncbi.nlm.nih.gov" => {
            let pmid = segs.first()?;
            return pmid
                .chars()
                .all(|c| c.is_ascii_digit())
                .then(|| Id::Pmid(pmid.to_string()));
        }
        "pmc.ncbi.nlm.nih.gov" | "ncbi.nlm.nih.gov" | "europepmc.org" => {
            let pmc = segs.iter().find(|s| {
                s.len() > 3
                    && s[..3].eq_ignore_ascii_case("pmc")
                    && s[3..].chars().all(|c| c.is_ascii_digit())
            })?;
            return Some(Id::Pmcid(pmc.to_ascii_uppercase()));
        }
        "sciencedirect.com" => {
            let i = segs.iter().position(|s| *s == "pii")?;
            return segs.get(i + 1).map(|p| Id::Pii(p.to_string()));
        }
        "nature.com" => {
            if let ["articles", id] = segs.as_slice() {
                return Some(Id::Doi(format!("10.1038/{id}")));
            }
        }
        _ => {}
    }
    // Some publishers carry the DOI as a query value (PLOS: ?id=10.1371/...).
    doi_in_path(path)
        .or_else(|| {
            url.query_pairs()
                .find_map(|(_, v)| doi_in_path(&format!("/{v}")))
        })
        .map(Id::Doi)
}

/// The first "10.<registrant>/<suffix>" in a path, minus publisher suffixes
/// such as "/full" or "/pdf" that follow the DOI.
fn doi_in_path(path: &str) -> Option<String> {
    let decoded = path.replace("%2F", "/").replace("%2f", "/");
    let start = decoded.find("/10.")? + 1;
    let doi = &decoded[start..];
    let (prefix, suffix) = doi.split_once('/')?;
    let registrant = prefix.strip_prefix("10.")?;
    if registrant.len() < 4 || !registrant.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    // Suffixes can contain slashes (10.1093/nar/gkab1112), so only known
    // trailing page-type segments are cut.
    let mut suffix = suffix.trim_end_matches('/');
    for tail in [
        "/full",
        "/abstract",
        "/pdf",
        "/epdf",
        "/html",
        "/fulltext",
        "/summary",
    ] {
        if let Some(s) = suffix.strip_suffix(tail) {
            suffix = s;
        }
    }
    (!suffix.is_empty()).then(|| format!("{prefix}/{suffix}"))
}

#[derive(Deserialize, Default)]
struct Work {
    id: Option<String>,
    doi: Option<String>,
    display_name: Option<String>,
    publication_year: Option<i64>,
    #[serde(default)]
    authorships: Vec<Authorship>,
    primary_location: Option<Location>,
    best_oa_location: Option<Location>,
    abstract_inverted_index: Option<std::collections::HashMap<String, Vec<usize>>>,
    ids: Option<WorkIds>,
    cited_by_count: Option<i64>,
    #[serde(default)]
    locations: Vec<Location>,
}

#[derive(Deserialize)]
struct Authorship {
    author: Option<Named>,
}

#[derive(Deserialize)]
struct Named {
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct Location {
    landing_page_url: Option<String>,
    pdf_url: Option<String>,
    license: Option<String>,
    source: Option<Named>,
}

#[derive(Deserialize)]
struct WorkIds {
    pmcid: Option<String>,
}

/// OpenAlex ships abstracts as word -> positions; rebuild the text.
fn rebuild_abstract(index: &std::collections::HashMap<String, Vec<usize>>) -> String {
    let len = index.values().flatten().max().map_or(0, |m| m + 1);
    let mut words = vec![""; len];
    for (word, positions) in index {
        for &p in positions {
            words[p] = word;
        }
    }
    words
        .into_iter()
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// arXiv id from a location's landing page, when the work has an arXiv copy.
fn arxiv_location(work: &Work) -> Option<String> {
    work.locations
        .iter()
        .chain(work.best_oa_location.iter())
        .find_map(|l| {
            let u = Url::parse(l.landing_page_url.as_deref()?).ok()?;
            match identify(&u)? {
                Id::Arxiv(id) => Some(id),
                _ => None,
            }
        })
}

/// Europe PMC full-text XML (JATS) to sectioned text.
fn jats_to_text(xml: &str) -> Option<String> {
    // JATS files often start with a DOCTYPE, which roxmltree refuses by default.
    let opts = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    };
    let doc = roxmltree::Document::parse_with_options(xml, opts).ok()?;
    let body = doc.descendants().find(|n| n.has_tag_name("body"))?;
    let mut out = String::new();
    for node in body.descendants() {
        let text = || {
            node.descendants()
                .filter(|d| d.is_text())
                .filter_map(|d| d.text())
                .collect::<String>()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        if node.has_tag_name("title") && node.parent().is_some_and(|p| p.has_tag_name("sec")) {
            out.push_str(&format!("\n## {}\n", text()));
        } else if node.has_tag_name("p") {
            out.push_str(&format!("\n{}\n", text()));
        }
    }
    let out = out.trim().to_string();
    (!out.is_empty()).then_some(out)
}

/// Readable article text from an HTML page, or None if it is a wall.
fn article_text(html: &str, url: &Url) -> Option<String> {
    let mut cursor = std::io::Cursor::new(html.as_bytes());
    let text = readability::extractor::extract(&mut cursor, url).ok()?.text;
    let text = text.trim().to_string();
    (walls::classify_content(&text).is_none()).then_some(text)
}

impl Scholarly {
    fn email(&self) -> String {
        self.ctx.config.settings.email.trim().to_string()
    }

    fn mailto(&self) -> String {
        let email = self.email();
        if email.is_empty() {
            String::new()
        } else {
            format!("mailto={email}")
        }
    }

    async fn openalex(&self, key: &str) -> Result<Option<Work>, SearchError> {
        let mailto = self.mailto();
        let url = format!(
            "{OPENALEX}/{key}{}",
            if mailto.is_empty() {
                String::new()
            } else {
                format!("?{mailto}")
            }
        );
        match get(&self.ctx, NAME, &url, "application/json").await {
            Ok(resp) => Ok(Some(resp.json().await?)),
            Err(SearchError::Api {
                status: Some(404), ..
            }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn pii_to_doi(&self, pii: &str) -> Result<Option<String>, SearchError> {
        #[derive(Deserialize)]
        struct R {
            message: M,
        }
        #[derive(Deserialize)]
        struct M {
            items: Vec<Item>,
        }
        #[derive(Deserialize)]
        struct Item {
            #[serde(rename = "DOI")]
            doi: String,
        }
        let mut url = format!("{CROSSREF}?filter=alternative-id:{pii}&rows=1&select=DOI");
        let mailto = self.mailto();
        if !mailto.is_empty() {
            url.push_str(&format!("&{mailto}"));
        }
        let r: R = get(&self.ctx, NAME, &url, "application/json")
            .await?
            .json()
            .await?;
        Ok(r.message.items.into_iter().next().map(|i| i.doi))
    }

    /// Crossref metadata for a DOI OpenAlex has not indexed yet.
    async fn crossref(&self, doi: &str) -> Result<Option<Work>, SearchError> {
        #[derive(Deserialize)]
        struct R {
            message: M,
        }
        #[derive(Deserialize)]
        struct M {
            #[serde(default)]
            title: Vec<String>,
            #[serde(default)]
            author: Vec<A>,
            #[serde(rename = "container-title", default)]
            container: Vec<String>,
            #[serde(rename = "URL")]
            url: Option<String>,
        }
        #[derive(Deserialize)]
        struct A {
            given: Option<String>,
            family: Option<String>,
        }
        let mailto = self.mailto();
        let url = format!(
            "{CROSSREF}/{doi}{}",
            if mailto.is_empty() {
                String::new()
            } else {
                format!("?{mailto}")
            }
        );
        let m = match get(&self.ctx, NAME, &url, "application/json").await {
            Ok(resp) => resp.json::<R>().await?.message,
            Err(SearchError::Api {
                status: Some(404), ..
            }) => return Ok(None),
            Err(e) => return Err(e),
        };
        Ok(Some(Work {
            doi: Some(format!("https://doi.org/{doi}")),
            display_name: m.title.into_iter().next(),
            authorships: m
                .author
                .into_iter()
                .map(|a| Authorship {
                    author: Some(Named {
                        display_name: Some(
                            [a.given, a.family]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(" "),
                        ),
                    }),
                })
                .collect(),
            primary_location: Some(Location {
                landing_page_url: m.url,
                pdf_url: None,
                license: None,
                source: m.container.into_iter().next().map(|c| Named {
                    display_name: Some(c),
                }),
            }),
            ..Work::default()
        }))
    }

    /// Unpaywall's best open-access copy. Needs a contact email.
    async fn unpaywall(&self, doi: &str) -> Result<Option<Location>, SearchError> {
        #[derive(Deserialize)]
        struct R {
            best_oa_location: Option<U>,
        }
        #[derive(Deserialize)]
        struct U {
            url_for_landing_page: Option<String>,
            url_for_pdf: Option<String>,
            license: Option<String>,
        }
        let email = self.email();
        if email.is_empty() {
            return Ok(None);
        }
        let url = format!("https://api.unpaywall.org/v2/{doi}?email={email}");
        let r: R = match get(&self.ctx, NAME, &url, "application/json").await {
            Ok(resp) => resp.json().await?,
            Err(SearchError::Api {
                status: Some(404), ..
            }) => return Ok(None),
            Err(e) => return Err(e),
        };
        Ok(r.best_oa_location.map(|u| Location {
            landing_page_url: u.url_for_landing_page,
            pdf_url: u.url_for_pdf,
            license: u.license,
            source: None,
        }))
    }

    async fn europepmc(&self, pmcid: &str) -> Option<String> {
        let url = format!("https://www.ebi.ac.uk/europepmc/webservices/rest/{pmcid}/fullTextXML");
        let xml = get(&self.ctx, NAME, &url, "application/xml")
            .await
            .ok()?
            .text()
            .await
            .ok()?;
        jats_to_text(&xml)
    }

    /// arXiv's LaTeXML pages are one long <article>; readability keeps only
    /// a section of it, so take the element whole.
    async fn arxiv_html(&self, url: &str) -> Option<String> {
        let html = get(&self.ctx, NAME, url, "text/html")
            .await
            .ok()?
            .text()
            .await
            .ok()?;
        let start = html.find("<article")?;
        let end = html.rfind("</article>").map_or(html.len(), |e| e + 10);
        let text = super::html_fragment_to_text(&html[start..end.max(start)]);
        walls::classify_content(&text).is_none().then_some(text)
    }

    async fn html_page(&self, url: &str) -> Option<String> {
        let parsed = Url::parse(url).ok()?;
        let html = get(&self.ctx, NAME, url, "text/html")
            .await
            .ok()?
            .text()
            .await
            .ok()?;
        article_text(&html, &parsed)
    }

    /// Full text and where it came from, trying the sources in order of
    /// quality. None leaves the result as metadata and abstract.
    async fn full_text(&self, work: &Work, arxiv: Option<&str>) -> Option<(String, String)> {
        let pmcid = work
            .ids
            .as_ref()
            .and_then(|i| i.pmcid.as_deref())
            .and_then(|p| p.rsplit('/').next())
            .map(|p| {
                // OpenAlex writes ".../pmc/articles/8371605"; Europe PMC wants "PMC8371605".
                if p.chars().all(|c| c.is_ascii_digit()) {
                    format!("PMC{p}")
                } else {
                    p.to_ascii_uppercase()
                }
            });
        if let Some(pmcid) = pmcid
            && let Some(t) = self.europepmc(&pmcid).await
        {
            return Some((t, format!("Europe PMC {pmcid}")));
        }
        if let Some(id) = arxiv {
            let url = format!("https://arxiv.org/html/{id}");
            if let Some(t) = self.arxiv_html(&url).await {
                return Some((t, url));
            }
        }
        let landing = work
            .best_oa_location
            .as_ref()?
            .landing_page_url
            .as_deref()?;
        let t = self.html_page(landing).await?;
        Some((t, landing.to_string()))
    }
}

fn render(work: &Work, full: Option<&(String, String)>) -> (String, String) {
    let title = work
        .display_name
        .clone()
        .unwrap_or_else(|| "Untitled work".into());
    let authors: Vec<&str> = work
        .authorships
        .iter()
        .filter_map(|a| a.author.as_ref()?.display_name.as_deref())
        .collect();
    let venue = work
        .primary_location
        .as_ref()
        .and_then(|l| l.source.as_ref()?.display_name.as_deref());
    let mut text = format!("# {title}\n\n");
    let mut byline = Vec::new();
    if !authors.is_empty() {
        let shown = authors
            .iter()
            .take(10)
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        byline.push(if authors.len() > 10 {
            format!("{shown}, et al.")
        } else {
            shown
        });
    }
    if let Some(v) = venue {
        byline.push(v.to_string());
    }
    if let Some(y) = work.publication_year {
        byline.push(y.to_string());
    }
    if let Some(d) = &work.doi {
        byline.push(d.clone());
    }
    text.push_str(&byline.join(" · "));
    text.push('\n');
    if let Some(oa) = &work.best_oa_location {
        let link = oa.pdf_url.as_deref().or(oa.landing_page_url.as_deref());
        if let Some(link) = link {
            text.push_str(&format!(
                "\nOpen access: {link}{}\n",
                oa.license
                    .as_deref()
                    .map(|l| format!(" ({l})"))
                    .unwrap_or_default()
            ));
        }
    }
    if let Some(index) = &work.abstract_inverted_index {
        text.push_str(&format!("\n## Abstract\n\n{}\n", rebuild_abstract(index)));
    }
    match full {
        Some((body, source)) => text.push_str(&format!("\n## Full text ({source})\n\n{body}\n")),
        None => text.push_str(
            "\nFull text: not available from an open source; the publisher page is not fetched.\n",
        ),
    }
    (title, text)
}

#[async_trait]
impl Route for Scholarly {
    fn name(&self) -> &'static str {
        NAME
    }

    fn matches(&self, url: &Url) -> bool {
        identify(url).is_some()
    }

    /// An identifier no index knows is worth one try through the chain.
    fn fallback_to_chain(&self) -> bool {
        true
    }

    async fn fetch(&self, url: &Url) -> Result<Vec<SearchResult>, SearchError> {
        let id = identify(url).ok_or_else(|| SearchError::InvalidInput {
            message: format!("no DOI, arXiv, PubMed or PMC id in {url}"),
        })?;
        let mut arxiv = None;
        let (key, doi) = match &id {
            Id::Doi(d) => (format!("doi:{d}"), Some(d.clone())),
            Id::Arxiv(a) => {
                arxiv = Some(a.clone());
                let d = format!("10.48550/arXiv.{a}");
                (format!("doi:{d}"), Some(d))
            }
            Id::Pmid(p) => (format!("pmid:{p}"), None),
            Id::Pmcid(p) => (format!("pmcid:{p}"), None),
            Id::Pii(p) => match self.pii_to_doi(p).await? {
                Some(d) => (format!("doi:{d}"), Some(d)),
                None => return Ok(vec![]),
            },
        };

        let mut work = match self.openalex(&key).await? {
            Some(w) => w,
            None => match &doi {
                Some(d) => match self.crossref(d).await? {
                    Some(w) => w,
                    None => return Ok(vec![]),
                },
                None => return Ok(vec![]),
            },
        };
        if work.best_oa_location.is_none()
            && let Some(d) = &doi
        {
            work.best_oa_location = self.unpaywall(d).await.unwrap_or(None);
        }
        let arxiv = arxiv.or_else(|| arxiv_location(&work));
        let full = self.full_text(&work, arxiv.as_deref()).await;
        let (title, text) = render(&work, full.as_ref());
        let oa = work.best_oa_location.as_ref();
        let extra = serde_json::json!({
            "doi": work.doi,
            "openalex_id": work.id,
            "arxiv_id": arxiv,
            "cited_by_count": work.cited_by_count,
            "oa_url": oa.and_then(|l| l.landing_page_url.clone()),
            "pdf_url": oa.and_then(|l| l.pdf_url.clone()),
            "license": oa.and_then(|l| l.license.clone()),
            "full_text_source": full.as_ref().map(|(_, s)| s.clone()),
        });
        Ok(vec![page(NAME, title, url.to_string(), text, extra)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(u: &str) -> Option<Id> {
        identify(&Url::parse(u).unwrap())
    }

    #[test]
    fn identifies_scholarly_urls() {
        assert_eq!(
            id("https://doi.org/10.1038/nature14539"),
            Some(Id::Doi("10.1038/nature14539".into()))
        );
        assert_eq!(
            id("https://link.springer.com/article/10.1007/s10994-021-05946-3"),
            Some(Id::Doi("10.1007/s10994-021-05946-3".into()))
        );
        assert_eq!(
            id("https://dl.acm.org/doi/10.1145/3442188.3445922"),
            Some(Id::Doi("10.1145/3442188.3445922".into()))
        );
        assert_eq!(
            id("https://onlinelibrary.wiley.com/doi/full/10.1002/anie.202100001"),
            Some(Id::Doi("10.1002/anie.202100001".into()))
        );
        assert_eq!(
            id("https://academic.oup.com/nar/article/10.1093/nar/gkab1112/pdf"),
            Some(Id::Doi("10.1093/nar/gkab1112".into()))
        );
        assert_eq!(
            id("https://www.nature.com/articles/s41586-021-03819-2"),
            Some(Id::Doi("10.1038/s41586-021-03819-2".into()))
        );
        assert_eq!(
            id("https://arxiv.org/abs/1706.03762v7"),
            Some(Id::Arxiv("1706.03762".into()))
        );
        assert_eq!(
            id("https://arxiv.org/pdf/1706.03762.pdf"),
            Some(Id::Arxiv("1706.03762".into()))
        );
        assert_eq!(
            id("https://arxiv.org/abs/hep-th/9901001"),
            Some(Id::Arxiv("hep-th/9901001".into()))
        );
        assert_eq!(
            id("https://pubmed.ncbi.nlm.nih.gov/34265844/"),
            Some(Id::Pmid("34265844".into()))
        );
        assert_eq!(
            id("https://pmc.ncbi.nlm.nih.gov/articles/PMC8371605/"),
            Some(Id::Pmcid("PMC8371605".into()))
        );
        assert_eq!(
            id("https://www.sciencedirect.com/science/article/pii/S0004370221000862"),
            Some(Id::Pii("S0004370221000862".into()))
        );
        assert_eq!(
            id("https://journals.plos.org/plosone/article?id=10.1371/journal.pone.0000308"),
            Some(Id::Doi("10.1371/journal.pone.0000308".into()))
        );
        assert_eq!(id("https://example.com/blog/10.5-reasons"), None);
        assert_eq!(id("https://arxiv.org/list/cs.AI/recent"), None);
        assert_eq!(id("https://www.nature.com/nature/volumes/600"), None);
    }

    #[test]
    fn abstract_rebuilds_from_inverted_index() {
        let mut idx = std::collections::HashMap::new();
        idx.insert("is".to_string(), vec![1]);
        idx.insert("Attention".to_string(), vec![0]);
        idx.insert("all".to_string(), vec![2]);
        assert_eq!(rebuild_abstract(&idx), "Attention is all");
    }

    #[test]
    fn jats_body_becomes_sectioned_text() {
        let xml = r#"<!DOCTYPE article PUBLIC "-//NLM//DTD JATS v1.3//EN" "JATS-archivearticle1-3.dtd">
<article><front><p>skip</p></front><body>
<sec><title>Intro</title><p>First <italic>para</italic>.</p></sec>
<sec><title>Methods</title><p>Second.</p><fig><caption><title>Fig</title></caption></fig></sec>
</body></article>"#;
        assert_eq!(
            jats_to_text(xml).unwrap(),
            "## Intro\n\nFirst para.\n\n## Methods\n\nSecond."
        );
    }
}
