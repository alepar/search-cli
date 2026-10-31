use hickory_resolver::Resolver;
use owo_colors::OwoColorize;
use serde::Serialize;
use std::io::IsTerminal;
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

const SMTP_TIMEOUT: Duration = Duration::from_secs(10);
const GREYLIST_DELAY: Duration = Duration::from_secs(5);

const DISPOSABLE_DOMAINS: &[&str] = &[
    "mailinator.com",
    "guerrillamail.com",
    "tempmail.com",
    "throwaway.email",
    "yopmail.com",
    "sharklasers.com",
    "guerrillamailblock.com",
    "grr.la",
    "dispostable.com",
    "trashmail.com",
    "mailnesia.com",
    "maildrop.cc",
    "discard.email",
    "tempail.com",
    "fakeinbox.com",
    "mailcatch.com",
    "temp-mail.org",
    "10minutemail.com",
    "mohmal.com",
    "burnermail.io",
    "inboxkitten.com",
    "emailondeck.com",
    "getnada.com",
    "tempr.email",
    "tmail.ws",
    "tmpmail.net",
    "tmpmail.org",
    "harakirimail.com",
    "mailsac.com",
    "spamgourmet.com",
    "jetable.org",
    "trash-mail.com",
    "mytemp.email",
    "boun.cr",
    "filzmail.com",
    "mailexpire.com",
    "tempinbox.com",
    "spamfree24.org",
    "mailforspam.com",
    "safetymail.info",
    "trashymail.com",
    "mailtemp.info",
    "temporarymail.com",
    "tempomail.fr",
    "mintemail.com",
    "discardmail.com",
    "mailnull.com",
    "spamhereplease.com",
];

#[derive(Debug, Clone, Serialize)]
pub struct VerifyResult {
    pub email: String,
    pub verdict: String,
    pub smtp_code: u16,
    pub mx_host: String,
    pub is_catch_all: bool,
    pub is_disposable: bool,
    pub suggestion: String,
}

pub async fn verify_emails(
    emails: &[String],
) -> Result<Vec<VerifyResult>, crate::errors::SearchError> {
    // Don't panic if /etc/resolv.conf is unreadable (sandbox/container) — that
    // would abort with a backtrace and break the --json envelope contract.
    let resolver = Resolver::builder_tokio()
        .map_err(|e| crate::errors::SearchError::Resolver(e.to_string()))?
        .build()
        .map_err(|e| crate::errors::SearchError::Resolver(e.to_string()))?;

    let mut results = Vec::with_capacity(emails.len());
    for email in emails {
        results.push(verify_one(&resolver, email).await);
    }
    Ok(results)
}

/// The domain of a syntactically plausible address, or None. The address is
/// written into the RCPT line, so anything that could break out of
/// `<...>` or start a new SMTP command is refused here.
fn syntax_domain(email: &str) -> Option<&str> {
    if email
        .chars()
        .any(|c| c.is_control() || c.is_whitespace() || c == '<' || c == '>')
    {
        return None;
    }
    let (local, domain) = email.split_once('@')?;
    if local.is_empty() || domain.is_empty() || !domain.contains('.') || domain.contains('@') {
        return None;
    }
    Some(domain)
}

async fn verify_one(resolver: &hickory_resolver::TokioResolver, email: &str) -> VerifyResult {
    let email = email.trim().to_lowercase();

    // Step 1: Syntax check
    let Some(domain) = syntax_domain(&email) else {
        return make_result(
            &email,
            "syntax_error",
            0,
            "",
            false,
            false,
            "Invalid email format.",
        );
    };

    let is_disposable = DISPOSABLE_DOMAINS.contains(&domain);

    // Step 2: MX lookup
    let mx_host = match resolve_mx(resolver, domain).await {
        Some(host) => host,
        None => {
            return make_result(
                &email,
                "unreachable",
                0,
                "",
                false,
                is_disposable,
                &format!("No MX records found for domain '{domain}'."),
            );
        }
    };

    // Step 3: Catch-all probe — use a clearly-fake local part
    let catch_all_addr = format!("xvfy-probe-7f3a9b@{domain}");
    let is_catch_all = matches!(
        smtp_probe(&mx_host, &catch_all_addr).await,
        SmtpResult::Accepted(_)
    );

    // Step 4: Real probe
    let result = smtp_probe(&mx_host, &email).await;

    // Step 5: Interpret result
    match result {
        SmtpResult::Accepted(code) => {
            if is_catch_all {
                make_result(
                    &email,
                    "catch_all",
                    code,
                    &mx_host,
                    true,
                    is_disposable,
                    "Domain accepts all addresses. Email format likely valid but unverifiable.",
                )
            } else {
                make_result(
                    &email,
                    "valid",
                    code,
                    &mx_host,
                    false,
                    is_disposable,
                    "Mailbox exists and accepts mail.",
                )
            }
        }
        SmtpResult::Rejected(code) => make_result(
            &email,
            "invalid",
            code,
            &mx_host,
            is_catch_all,
            is_disposable,
            "Mailbox does not exist.",
        ),
        SmtpResult::Greylisted(code) => {
            // Retry once after delay
            tokio::time::sleep(GREYLIST_DELAY).await;
            match smtp_probe(&mx_host, &email).await {
                SmtpResult::Accepted(c) => {
                    if is_catch_all {
                        make_result(&email, "catch_all", c, &mx_host, true, is_disposable,
                            "Domain accepts all addresses. Email format likely valid but unverifiable.")
                    } else {
                        make_result(
                            &email,
                            "valid",
                            c,
                            &mx_host,
                            false,
                            is_disposable,
                            "Mailbox exists and accepts mail (passed greylist).",
                        )
                    }
                }
                SmtpResult::Rejected(c) => make_result(
                    &email,
                    "invalid",
                    c,
                    &mx_host,
                    is_catch_all,
                    is_disposable,
                    "Mailbox does not exist.",
                ),
                SmtpResult::Refused(c, text) => {
                    refused_result(&email, c, &text, &mx_host, is_catch_all, is_disposable)
                }
                _ => make_result(
                    &email,
                    "unreachable",
                    code,
                    &mx_host,
                    is_catch_all,
                    is_disposable,
                    "Server greylisted the request and did not respond on retry.",
                ),
            }
        }
        SmtpResult::Refused(code, text) => {
            refused_result(&email, code, &text, &mx_host, is_catch_all, is_disposable)
        }
        SmtpResult::Timeout => make_result(
            &email,
            "timeout",
            0,
            &mx_host,
            is_catch_all,
            is_disposable,
            "SMTP server did not respond within timeout.",
        ),
        SmtpResult::Error(msg) => make_result(
            &email,
            "unreachable",
            0,
            &mx_host,
            is_catch_all,
            is_disposable,
            &format!("Connection failed: {msg}"),
        ),
    }
}

async fn resolve_mx(resolver: &hickory_resolver::TokioResolver, domain: &str) -> Option<String> {
    // hickory 0.26: mx_lookup returns a plain Lookup; MX rdata comes out of
    // the answer records.
    use hickory_resolver::proto::rr::RData;
    match resolver.mx_lookup(domain).await {
        Ok(lookup) => lookup
            .answers()
            .iter()
            .filter_map(|rec| match &rec.data {
                RData::MX(mx) => Some(mx),
                _ => None,
            })
            .min_by_key(|mx| mx.preference)
            .map(|mx| mx.exchange.to_string().trim_end_matches('.').to_string()),
        Err(_) => None,
    }
}

/// A rejection that says nothing about the mailbox (policy, reputation,
/// unknown code): the probe was refused, so the address is unverified.
fn refused_result(
    email: &str,
    code: u16,
    text: &str,
    mx_host: &str,
    is_catch_all: bool,
    is_disposable: bool,
) -> VerifyResult {
    make_result(
        email,
        "unreachable",
        code,
        mx_host,
        is_catch_all,
        is_disposable,
        &format!("Server refused the probe without confirming the mailbox: {text}"),
    )
}

enum SmtpResult {
    Accepted(u16),
    /// The mailbox does not exist (550/551/553 without a policy status).
    Rejected(u16),
    Greylisted(u16),
    /// Any other final answer: policy blocks, unknown codes. Carries the
    /// reply text so the caller can show why.
    Refused(u16, String),
    Timeout,
    Error(String),
}

/// Upper bound on one whole probe, however the server paces its replies.
const PROBE_DEADLINE: Duration = Duration::from_secs(30);
/// RFC 5321 caps a reply line at 512 octets; allow slack, refuse floods.
const MAX_REPLY_LINE: u64 = 4096;
const MAX_REPLY_LINES: usize = 64;

async fn smtp_probe(mx_host: &str, email: &str) -> SmtpResult {
    // An MX record is attacker-controlled input: whoever owns the domain
    // decides which host we connect to. Without this the CLI probes whatever
    // internal address a hostile domain names, and the SMTP response codes
    // report back what is listening.
    if let Err(e) = crate::guard::assert_public_host(mx_host, 25).await {
        return SmtpResult::Error(e.to_string());
    }

    let addr = format!("{mx_host}:25");

    let stream = match timeout(SMTP_TIMEOUT, TcpStream::connect(&addr)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return SmtpResult::Error(e.to_string()),
        Err(_) => return SmtpResult::Timeout,
    };

    timeout(PROBE_DEADLINE, smtp_session(stream, email))
        .await
        .unwrap_or(SmtpResult::Timeout)
}

/// One SMTP conversation up to RCPT, generic over the stream for tests.
async fn smtp_session<S: AsyncRead + AsyncWrite + Unpin>(stream: S, email: &str) -> SmtpResult {
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);

    match read_reply(&mut reader).await {
        Ok(r) if r.code < 400 => {}
        Ok(r) => return SmtpResult::Error(format!("greeting refused: {}", r.text)),
        Err(e) => return e.into_result("no greeting"),
    }
    for (cmd, what) in [
        ("EHLO verify.local\r\n", "EHLO"),
        ("MAIL FROM:<>\r\n", "MAIL FROM"),
    ] {
        match command(&mut writer, &mut reader, cmd).await {
            Ok(r) if r.code < 400 => {}
            Ok(r) => return SmtpResult::Error(format!("{what} refused: {}", r.text)),
            Err(e) => return e.into_result(what),
        }
    }

    // RCPT TO — this is the actual probe
    let reply = match command(&mut writer, &mut reader, &format!("RCPT TO:<{email}>\r\n")).await {
        Ok(r) => r,
        Err(e) => return e.into_result("RCPT TO"),
    };

    // Always QUIT cleanly
    let _ = timeout(Duration::from_secs(2), writer.write_all(b"QUIT\r\n")).await;

    classify_rcpt(&reply)
}

fn classify_rcpt(reply: &Reply) -> SmtpResult {
    match reply.code {
        250 | 251 => SmtpResult::Accepted(reply.code),
        421 | 450 | 451 | 452 => SmtpResult::Greylisted(reply.code),
        550 | 551 | 553 if !is_policy_status(&reply.text) => SmtpResult::Rejected(reply.code),
        code => SmtpResult::Refused(code, reply.text.clone()),
    }
}

/// Enhanced status class 5.7.x (RFC 3463) is a security or policy refusal
/// of the sender, e.g. "550 5.7.1 Client host blocked", not a missing mailbox.
fn is_policy_status(text: &str) -> bool {
    text.get(4..)
        .and_then(|rest| rest.split_whitespace().next())
        .is_some_and(|status| status.starts_with("5.7."))
}

struct Reply {
    code: u16,
    /// Final line of the reply, trimmed.
    text: String,
}

enum ReadError {
    Timeout,
    Closed,
    Malformed(&'static str),
    Io(String),
}

impl ReadError {
    fn into_result(self, stage: &str) -> SmtpResult {
        match self {
            Self::Timeout => SmtpResult::Timeout,
            Self::Closed => SmtpResult::Error(format!("{stage}: connection closed")),
            Self::Malformed(why) => SmtpResult::Error(format!("{stage}: {why}")),
            Self::Io(e) => SmtpResult::Error(format!("{stage}: {e}")),
        }
    }
}

async fn command<W, R>(writer: &mut W, reader: &mut R, cmd: &str) -> Result<Reply, ReadError>
where
    W: AsyncWrite + Unpin,
    R: AsyncBufRead + Unpin,
{
    match timeout(SMTP_TIMEOUT, writer.write_all(cmd.as_bytes())).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(ReadError::Io(e.to_string())),
        Err(_) => return Err(ReadError::Timeout),
    }
    read_reply(reader).await
}

/// Read one (possibly multi-line) reply. Continuation lines carry `-` after
/// the code ("250-PIPELINING"); the final line carries a space.
async fn read_reply<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Reply, ReadError> {
    for _ in 0..MAX_REPLY_LINES {
        let mut buf = Vec::new();
        let n = timeout(
            SMTP_TIMEOUT,
            (&mut *reader)
                .take(MAX_REPLY_LINE)
                .read_until(b'\n', &mut buf),
        )
        .await
        .map_err(|_| ReadError::Timeout)?
        .map_err(|e| ReadError::Io(e.to_string()))?;
        if n == 0 {
            return Err(ReadError::Closed);
        }
        if n as u64 == MAX_REPLY_LINE && !buf.ends_with(b"\n") {
            return Err(ReadError::Malformed("reply line too long"));
        }
        let line = String::from_utf8_lossy(&buf);
        if line.as_bytes().get(3) == Some(&b'-') {
            continue;
        }
        return Ok(Reply {
            code: parse_code(&line),
            text: line.trim_end().to_string(),
        });
    }
    Err(ReadError::Malformed("too many reply lines"))
}

fn parse_code(line: &str) -> u16 {
    line.get(..3).and_then(|s| s.parse().ok()).unwrap_or(0)
}

fn make_result(
    email: &str,
    verdict: &str,
    smtp_code: u16,
    mx_host: &str,
    is_catch_all: bool,
    is_disposable: bool,
    suggestion: &str,
) -> VerifyResult {
    VerifyResult {
        email: email.to_string(),
        verdict: verdict.to_string(),
        smtp_code,
        mx_host: mx_host.to_string(),
        is_catch_all,
        is_disposable,
        suggestion: suggestion.to_string(),
    }
}

pub fn render_table(results: &[VerifyResult]) {
    let use_color = std::io::stdout().is_terminal();

    if use_color {
        eprintln!("\n{}  Email Verification\n", "search".bold().cyan());
    }

    for r in results {
        let verdict_display = if use_color {
            match r.verdict.as_str() {
                "valid" => format!("{}", "VALID".green().bold()),
                "invalid" => format!("{}", "INVALID".red().bold()),
                "catch_all" => format!("{}", "CATCH-ALL".yellow().bold()),
                "unreachable" => format!("{}", "UNREACHABLE".red()),
                "timeout" => format!("{}", "TIMEOUT".yellow()),
                "syntax_error" => format!("{}", "SYNTAX ERROR".red()),
                _ => r.verdict.clone(),
            }
        } else {
            r.verdict.to_uppercase()
        };

        let email_display = if use_color {
            r.email.bold().to_string()
        } else {
            r.email.clone()
        };

        println!("  {} -> {}", email_display, verdict_display);
        if !r.mx_host.is_empty() {
            if use_color {
                println!("    {} {}", "MX:".dimmed(), r.mx_host.dimmed());
            } else {
                println!("    MX: {}", r.mx_host);
            }
        }
        if use_color {
            println!("    {}", r.suggestion.dimmed());
        } else {
            println!("    {}", r.suggestion);
        }
        println!();
    }

    let valid = results.iter().filter(|r| r.verdict == "valid").count();
    let total = results.len();
    if use_color {
        eprintln!("  {}/{} verified as valid", valid.to_string().bold(), total);
    } else {
        eprintln!("  {}/{} verified as valid", valid, total);
    }
    eprintln!();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run one session against a scripted server that has all its replies
    /// queued, then hangs up. The server half drains the client's commands so writes succeed.
    async fn session(script: String) -> SmtpResult {
        let (client, server) = tokio::io::duplex(1 << 20);
        tokio::spawn(async move {
            let (mut r, mut w) = tokio::io::split(server);
            w.write_all(script.as_bytes()).await.unwrap();
            // EOF after the script, like a server hanging up.
            w.shutdown().await.unwrap();
            let mut sink = Vec::new();
            let _ = r.read_to_end(&mut sink).await;
        });
        timeout(
            Duration::from_secs(2),
            smtp_session(client, "user@example.com"),
        )
        .await
        .expect("session must not wait on a complete reply")
    }

    const PRELUDE: &str =
        "220 mx ready\r\n250-mx hello\r\n250-PIPELINING\r\n250 SIZE 1000\r\n250 ok\r\n";

    #[tokio::test]
    async fn multi_line_ehlo_reply_does_not_stall() {
        let r = session(format!("{PRELUDE}550 5.1.1 no such user\r\n")).await;
        assert!(matches!(r, SmtpResult::Rejected(550)));
    }

    #[tokio::test]
    async fn accepted_rcpt() {
        let r = session(format!("{PRELUDE}250 2.1.5 ok\r\n")).await;
        assert!(matches!(r, SmtpResult::Accepted(250)));
    }

    #[tokio::test]
    async fn policy_rejection_is_not_a_missing_mailbox() {
        let r = session(format!("{PRELUDE}550 5.7.1 Client host blocked\r\n")).await;
        assert!(matches!(r, SmtpResult::Refused(550, _)));
        let r = session(format!("{PRELUDE}554 transaction failed\r\n")).await;
        assert!(matches!(r, SmtpResult::Refused(554, _)));
    }

    #[tokio::test]
    async fn closed_connection_at_rcpt_is_not_invalid() {
        let r = session(PRELUDE.to_string()).await;
        assert!(
            matches!(r, SmtpResult::Error(_)),
            "EOF must not map to Rejected"
        );
    }

    #[tokio::test]
    async fn endless_continuation_is_cut_off() {
        let flood = "250-x\r\n".repeat(MAX_REPLY_LINES + 1);
        let r = session(format!("220 hi\r\n{flood}")).await;
        assert!(matches!(r, SmtpResult::Error(_)));
    }

    #[test]
    fn syntax_refuses_smtp_injection() {
        assert_eq!(syntax_domain("a@b.com"), Some("b.com"));
        assert_eq!(syntax_domain("a\r\nRSET@b.com"), None);
        assert_eq!(syntax_domain("a>@b.com"), None);
        assert_eq!(syntax_domain("a@b@c.com"), None);
        assert_eq!(syntax_domain("a b@c.com"), None);
        assert_eq!(syntax_domain("@c.com"), None);
    }
}
