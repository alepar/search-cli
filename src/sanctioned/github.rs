//! GitHub repos, files and issues/PRs through the REST API: clean markdown
//! instead of page chrome. Unauthenticated callers get 60 requests/hour;
//! GITHUB_TOKEN (or GH_TOKEN) raises that to 5,000.

use super::{host_is, page, send, Route};
use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::SearchResult;
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;
use url::Url;

const NAME: &str = "github";
const API: &str = "https://api.github.com";
const MAX_COMMENTS: usize = 30;

pub struct GitHub {
    ctx: Arc<AppContext>,
}

impl GitHub {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }
}

#[derive(Debug, PartialEq)]
enum Target {
    Repo {
        owner: String,
        repo: String,
    },
    File {
        owner: String,
        repo: String,
        git_ref: String,
        path: String,
    },
    Issue {
        owner: String,
        repo: String,
        number: u64,
    },
}

fn parse(url: &Url) -> Option<Target> {
    let host = url.host_str()?.to_ascii_lowercase();
    if host != "github.com" && host != "www.github.com" {
        return None;
    }
    let segs: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let (owner, repo) = (segs.first()?.to_string(), segs.get(1)?.to_string());
    let repo = repo.trim_end_matches(".git").to_string();
    match segs.get(2).copied() {
        None => Some(Target::Repo { owner, repo }),
        Some("blob") if segs.len() >= 5 => Some(Target::File {
            owner,
            repo,
            git_ref: segs[3].to_string(),
            path: segs[4..].join("/"),
        }),
        Some("issues" | "pull") => Some(Target::Issue {
            owner,
            repo,
            number: segs.get(3)?.parse().ok()?,
        }),
        _ => None,
    }
}

#[derive(Deserialize)]
struct Repo {
    full_name: String,
    html_url: String,
    description: Option<String>,
    stargazers_count: Option<u64>,
    language: Option<String>,
    #[serde(default)]
    topics: Vec<String>,
    license: Option<License>,
}

#[derive(Deserialize)]
struct License {
    spdx_id: Option<String>,
}

#[derive(Deserialize)]
struct Issue {
    title: String,
    html_url: String,
    body: Option<String>,
    state: String,
    user: Option<User>,
    pull_request: Option<serde_json::Value>,
    created_at: Option<String>,
}

#[derive(Deserialize)]
struct Comment {
    body: Option<String>,
    user: Option<User>,
}

#[derive(Deserialize)]
struct User {
    login: String,
}

fn login(u: &Option<User>) -> &str {
    u.as_ref().map_or("ghost", |u| u.login.as_str())
}

impl GitHub {
    fn request(&self, url: &str, accept: &str) -> reqwest::RequestBuilder {
        let mut req = self
            .ctx
            .client
            .get(url)
            .header("Accept", accept)
            .header("X-GitHub-Api-Version", "2022-11-28");
        let token = crate::providers::resolve_key(
            &crate::providers::resolve_key("", "GH_TOKEN"),
            "GITHUB_TOKEN",
        );
        if !token.is_empty() {
            req = req.bearer_auth(token);
        }
        req
    }

    async fn json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T, SearchError> {
        let req = self.request(url, "application/vnd.github+json");
        Ok(send(NAME, url, req).await?.json().await?)
    }

    async fn raw(&self, url: &str) -> Result<String, SearchError> {
        let req = self.request(url, "application/vnd.github.raw+json");
        Ok(send(NAME, url, req).await?.text().await?)
    }
}

#[async_trait]
impl Route for GitHub {
    fn name(&self) -> &'static str {
        NAME
    }

    fn matches(&self, url: &Url) -> bool {
        host_is(url, "github.com") && parse(url).is_some()
    }

    /// github.com pages are not walled: when the API is rate-limited (60/h
    /// without a token), the page itself is still worth a try.
    fn fallback_to_chain(&self) -> bool {
        true
    }

    async fn fetch(&self, url: &Url) -> Result<Vec<SearchResult>, SearchError> {
        let target = parse(url).ok_or_else(|| SearchError::InvalidInput {
            message: format!("not a GitHub repo, file or issue URL: {url}"),
        })?;
        let result = match target {
            Target::Repo { owner, repo } => {
                let r: Repo = self.json(&format!("{API}/repos/{owner}/{repo}")).await?;
                // A repo without a README is still a repo.
                let readme = self
                    .raw(&format!("{API}/repos/{owner}/{repo}/readme"))
                    .await
                    .unwrap_or_default();
                let mut text = format!("# {}\n", r.full_name);
                if let Some(d) = &r.description {
                    text.push_str(&format!("\n{d}\n"));
                }
                text.push_str(&format!("\n{readme}"));
                let extra = serde_json::json!({
                    "stars": r.stargazers_count,
                    "language": r.language,
                    "topics": r.topics,
                    "license": r.license.and_then(|l| l.spdx_id),
                });
                page(NAME, r.full_name, r.html_url, text, extra)
            }
            Target::File {
                owner,
                repo,
                git_ref,
                path,
            } => {
                let text = self
                    .raw(&format!(
                        "{API}/repos/{owner}/{repo}/contents/{path}?ref={git_ref}"
                    ))
                    .await?;
                let extra =
                    serde_json::json!({ "repo": format!("{owner}/{repo}"), "ref": git_ref });
                page(NAME, path, url.to_string(), text, extra)
            }
            Target::Issue {
                owner,
                repo,
                number,
            } => {
                let base = format!("{API}/repos/{owner}/{repo}/issues/{number}");
                let issue: Issue = self.json(&base).await?;
                let comments: Vec<Comment> = self
                    .json(&format!("{base}/comments?per_page={MAX_COMMENTS}"))
                    .await?;
                let kind = if issue.pull_request.is_some() {
                    "Pull request"
                } else {
                    "Issue"
                };
                let mut text = format!(
                    "# {}\n\n{kind} #{number} in {owner}/{repo} by {} ({})\n\n{}\n",
                    issue.title,
                    login(&issue.user),
                    issue.state,
                    issue.body.as_deref().unwrap_or_default(),
                );
                for c in &comments {
                    text.push_str(&format!(
                        "\n## Comment by {}\n\n{}\n",
                        login(&c.user),
                        c.body.as_deref().unwrap_or_default()
                    ));
                }
                let extra = serde_json::json!({
                    "state": issue.state,
                    "comments_included": comments.len(),
                    "created_at": issue.created_at,
                });
                page(NAME, issue.title, issue.html_url, text, extra)
            }
        };
        Ok(vec![result])
    }
}

#[cfg(test)]
mod tests {
    use super::{parse, Target};
    use url::Url;

    fn p(u: &str) -> Option<Target> {
        parse(&Url::parse(u).unwrap())
    }

    #[test]
    fn parses_repo_file_and_issue_urls() {
        assert_eq!(
            p("https://github.com/rust-lang/rust"),
            Some(Target::Repo {
                owner: "rust-lang".into(),
                repo: "rust".into()
            })
        );
        assert_eq!(
            p("https://github.com/rust-lang/rust/blob/master/src/doc/README.md"),
            Some(Target::File {
                owner: "rust-lang".into(),
                repo: "rust".into(),
                git_ref: "master".into(),
                path: "src/doc/README.md".into()
            })
        );
        assert_eq!(
            p("https://github.com/rust-lang/rust/pull/12345"),
            Some(Target::Issue {
                owner: "rust-lang".into(),
                repo: "rust".into(),
                number: 12345
            })
        );
        assert_eq!(p("https://github.com/rust-lang/rust/actions"), None);
        assert_eq!(p("https://github.com/rust-lang"), None);
        assert_eq!(p("https://gist.github.com/a/b"), None);
    }
}
