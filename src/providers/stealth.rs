use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::{SearchOpts, SearchResult};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use url::Url;
use wreq_util::Profile;

pub struct Stealth {
    _ctx: Arc<AppContext>,
}

impl Stealth {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { _ctx: ctx }
    }

    /// A wreq client with a current Chrome TLS/HTTP2 fingerprint. The
    /// emulation profile supplies Chrome's own default headers, so nothing
    /// is layered on top: hand-written client hints went stale and
    /// contradicted the profile. Redirects are followed by hand in
    /// [`Self::fetch`], so each hop can pass the network guard.
    fn build_client() -> Result<wreq::Client, SearchError> {
        wreq::Client::builder()
            .emulation(Profile::Chrome149)
            .redirect(wreq::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(SearchError::Wreq)
    }

    /// GET `start`, following up to MAX_REDIRECTS redirects. Returns the
    /// final URL and the body. The guard in main checks only the URL the
    /// caller gave; a public page must not redirect the local fetcher to a
    /// private address. When the start URL is itself private, the caller
    /// passed --allow-private, and the hops are not checked either.
    async fn fetch(client: &wreq::Client, start: &Url) -> Result<(Url, Vec<u8>), SearchError> {
        let check_hops = crate::guard::assert_public_url(start.as_str())
            .await
            .is_ok();
        let mut url = start.clone();
        for _ in 0..=MAX_REDIRECTS {
            let resp = client.get(url.as_str()).send().await?;
            let status = resp.status();
            if status.is_redirection() {
                let next = resp
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|loc| url.join(loc).ok())
                    .ok_or_else(|| {
                        api_error(status.as_u16(), "redirect without a valid Location")
                    })?;
                if !matches!(next.scheme(), "http" | "https") {
                    return Err(api_error(status.as_u16(), "redirect to a non-http URL"));
                }
                if check_hops {
                    crate::guard::assert_public_url(next.as_str()).await?;
                }
                url = next;
                continue;
            }
            if !status.is_success() {
                return Err(api_error(
                    status.as_u16(),
                    &format!("HTTP {}", status.as_u16()),
                ));
            }
            let body = resp.bytes().await?;
            return Ok((url, body.to_vec()));
        }
        Err(api_error(0, "too many redirects"))
    }

    pub async fn scrape_url(&self, url_str: &str) -> Result<Vec<SearchResult>, SearchError> {
        let client = Self::build_client()?;
        let url = Url::parse(url_str).map_err(|e| SearchError::InvalidInput {
            message: format!("Invalid URL: {e}"),
        })?;

        let (final_url, mut html_bytes) = Self::fetch(&client, &url).await?;
        // Bound the parse: readability's cost grows with the page.
        html_bytes.truncate(MAX_PARSE_BYTES);
        let html = String::from_utf8_lossy(&html_bytes).into_owned();

        let (title, text) = extract_article(html, final_url.clone()).await;

        if text.trim().is_empty() {
            return Err(SearchError::Api {
                provider: "stealth",
                code: "extraction_error",
                message: "Page returned no extractable text content".to_string(),
                status: None,
            });
        }

        Ok(vec![SearchResult {
            title,
            url: final_url.to_string(),
            snippet: text,
            source: "stealth".to_string(),
            published: None,
            image_url: None,
            extra: None,
        }])
    }
}

const MAX_REDIRECTS: usize = 10;
/// Pages beyond this are truncated before parsing.
const MAX_PARSE_BYTES: usize = 8 * 1024 * 1024;
/// Readability recurses over the DOM; deeply nested pages overflow a
/// default 2 MiB stack and abort the process. Reserved, not committed.
const PARSE_STACK_BYTES: usize = 256 * 1024 * 1024;

fn api_error(status: u16, message: &str) -> SearchError {
    SearchError::Api {
        provider: "stealth",
        code: "api_error",
        message: message.to_string(),
        status: (status != 0).then_some(status),
    }
}

/// Readability first, plain-text fallback. Runs on its own thread with a
/// large stack: the parse is synchronous, so on the runtime it would block
/// past the caller's timeout, and a deep DOM would overflow the stack. If
/// the caller's timeout fires, the thread is abandoned.
async fn extract_article(html: String, url: Url) -> (String, String) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("readability".into())
        .stack_size(PARSE_STACK_BYTES)
        .spawn(move || {
            let mut cursor = std::io::Cursor::new(html.as_bytes());
            let out = match readability::extractor::extract(&mut cursor, &url) {
                Ok(article) if !article.text.trim().is_empty() => {
                    let title = if article.title.is_empty() {
                        url.to_string()
                    } else {
                        article.title
                    };
                    (title, article.text)
                }
                _ => (url.to_string(), super::html_to_text(&html)),
            };
            let _ = tx.send(out);
        });
    match spawned {
        Ok(_) => rx.await.unwrap_or_default(),
        Err(_) => (String::new(), String::new()),
    }
}

#[async_trait]
impl super::Provider for Stealth {
    fn name(&self) -> &'static str {
        "stealth"
    }

    fn capabilities(&self) -> &[&'static str] {
        &["scrape", "extract"]
    }

    fn env_keys(&self) -> &[&'static str] {
        &[] // No API key needed
    }

    fn is_configured(&self) -> bool {
        true // No API key needed — local scraper
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    async fn search(
        &self,
        _query: &str,
        _count: usize,
        _opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        Ok(vec![])
    }

    async fn search_news(
        &self,
        _query: &str,
        _count: usize,
        _opts: &SearchOpts,
    ) -> Result<Vec<SearchResult>, SearchError> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Serve two requests: a 302 to /final, then the page.
    async fn redirecting_server() -> std::net::SocketAddr {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let body = format!(
                "<html><head><title>Final</title></head><body><p>{}</p></body></html>",
                "Redirected page content. ".repeat(40)
            );
            let replies = [
                "HTTP/1.1 302 Found\r\nlocation: /final\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    .to_string(),
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                ),
            ];
            for reply in replies {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        addr
    }

    #[tokio::test]
    async fn follows_redirects_and_reports_the_final_url() {
        let addr = redirecting_server().await;
        let client = Stealth::build_client().unwrap();
        let start = Url::parse(&format!("http://{addr}/start")).unwrap();
        let (url, body) = Stealth::fetch(&client, &start).await.unwrap();
        assert_eq!(url.path(), "/final");
        assert!(String::from_utf8_lossy(&body).contains("Redirected page content"));
    }

    #[tokio::test]
    async fn deeply_nested_page_does_not_abort_the_process() {
        let depth = 5_000;
        let html = format!(
            "<html><body>{}<p>deep text</p>{}</body></html>",
            "<div>".repeat(depth),
            "</div>".repeat(depth)
        );
        let url = Url::parse("https://example.com/").unwrap();
        // Reaching the assertion at all is the test: before, this overflowed
        // the runtime thread's stack and aborted.
        let (_title, _text) = extract_article(html, url).await;
    }
}
