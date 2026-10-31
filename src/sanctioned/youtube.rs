//! YouTube videos: title and channel from oEmbed, transcript from yt-dlp when
//! it is on PATH. The watch page carries no transcript, and YouTube's caption
//! endpoint now wants a proof-of-origin token that yt-dlp knows how to get.

use super::{get, page, Route};
use crate::context::AppContext;
use crate::errors::SearchError;
use crate::types::SearchResult;
use async_trait::async_trait;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use url::Url;

const NAME: &str = "youtube";

pub struct YouTube {
    ctx: Arc<AppContext>,
}

impl YouTube {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }
}

fn video_id(url: &Url) -> Option<String> {
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.trim_start_matches("www.").trim_start_matches("m.");
    let segs: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let id = match (host, segs.as_slice()) {
        ("youtu.be", [id]) => id.to_string(),
        ("youtube.com", ["watch"]) => url
            .query_pairs()
            .find(|(k, _)| k == "v")
            .map(|(_, v)| v.into_owned())?,
        ("youtube.com", ["shorts" | "live" | "embed", id]) => id.to_string(),
        _ => return None,
    };
    let valid = id.len() == 11
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then_some(id)
}

#[derive(Deserialize)]
struct OEmbed {
    title: String,
    author_name: Option<String>,
}

/// WebVTT to plain text. Auto-captions repeat each line as it scrolls, so
/// consecutive duplicates are dropped; inline timing and style tags go too.
fn vtt_to_text(vtt: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for raw in vtt.lines() {
        let line = raw.trim();
        if line.is_empty()
            || line == "WEBVTT"
            || line.contains("-->")
            || line.starts_with("Kind:")
            || line.starts_with("Language:")
            || line.starts_with("NOTE")
            || line.chars().all(|c| c.is_ascii_digit())
        {
            continue;
        }
        let mut clean = String::with_capacity(line.len());
        let mut in_tag = false;
        for c in line.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                _ if !in_tag => clean.push(c),
                _ => {}
            }
        }
        let clean = super::decode_entities(clean.trim());
        if !clean.is_empty() && lines.last() != Some(&clean) {
            lines.push(clean);
        }
    }
    lines.join(" ")
}

/// Prefer human-made English captions over auto-generated ones.
fn pick_subtitle(dir: &Path, id: &str) -> Option<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "vtt"))
        .collect();
    files.sort();
    let exact = dir.join(format!("{id}.en.vtt"));
    if files.contains(&exact) {
        return Some(exact);
    }
    files.into_iter().next()
}

/// Run yt-dlp for subtitles only. Err carries a short reason for the agent.
fn transcript(id: &str) -> Result<String, String> {
    let dir = std::env::temp_dir().join(format!("search-cli-yt-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;
    let out = std::process::Command::new("yt-dlp")
        .args([
            "--skip-download",
            "--write-subs",
            "--write-auto-subs",
            "--sub-langs",
            "en,en-.*,en.*",
            "--sub-format",
            "vtt",
            "--no-progress",
            "--no-warnings",
            "-o",
        ])
        .arg(dir.join("%(id)s.%(ext)s"))
        .arg("--")
        .arg(format!("https://www.youtube.com/watch?v={id}"))
        .output();
    let result = match out {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err("yt-dlp is not on PATH; install it for transcripts".to_string())
        }
        Err(e) => Err(format!("yt-dlp failed to start: {e}")),
        Ok(o) => match pick_subtitle(&dir, id) {
            Some(file) => std::fs::read_to_string(file)
                .map(|v| vtt_to_text(&v))
                .map_err(|e| e.to_string()),
            None => {
                let err = String::from_utf8_lossy(&o.stderr);
                let last = err.lines().rev().find(|l| !l.trim().is_empty());
                Err(format!(
                    "no English captions via yt-dlp{}",
                    last.map(|l| format!(": {}", l.trim())).unwrap_or_default()
                ))
            }
        },
    };
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[async_trait]
impl Route for YouTube {
    fn name(&self) -> &'static str {
        NAME
    }

    fn matches(&self, url: &Url) -> bool {
        video_id(url).is_some()
    }

    async fn fetch(&self, url: &Url) -> Result<Vec<SearchResult>, SearchError> {
        let id = video_id(url).ok_or_else(|| SearchError::InvalidInput {
            message: format!("not a YouTube video URL: {url}"),
        })?;
        let watch = format!("https://www.youtube.com/watch?v={id}");
        let oembed_url = format!("https://www.youtube.com/oembed?format=json&url={watch}");
        let meta: OEmbed = get(&self.ctx, NAME, &oembed_url, "application/json")
            .await?
            .json()
            .await?;
        let tid = id.clone();
        let transcript = tokio::task::spawn_blocking(move || transcript(&tid))
            .await
            .unwrap_or_else(|e| Err(format!("transcript task failed: {e}")));

        let channel = meta.author_name.unwrap_or_default();
        let mut text = format!("# {}\n\nChannel: {channel}\n", meta.title);
        let transcript_note = match &transcript {
            Ok(t) => {
                text.push_str(&format!("\n## Transcript\n\n{t}\n"));
                serde_json::Value::Null
            }
            Err(reason) => serde_json::json!(reason),
        };
        let extra = serde_json::json!({
            "video_id": id,
            "channel": channel,
            "transcript": transcript.is_ok(),
            "transcript_unavailable": transcript_note,
        });
        Ok(vec![page(NAME, meta.title, watch, text, extra)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_ids_parse_from_common_url_shapes() {
        let id = |u: &str| video_id(&Url::parse(u).unwrap());
        let want = Some("dQw4w9WgXcQ".to_string());
        assert_eq!(id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1s"), want);
        assert_eq!(id("https://youtu.be/dQw4w9WgXcQ"), want);
        assert_eq!(id("https://m.youtube.com/shorts/dQw4w9WgXcQ"), want);
        assert_eq!(id("https://www.youtube.com/@channel"), None);
        assert_eq!(id("https://www.youtube.com/watch?v=short"), None);
    }

    #[test]
    fn vtt_drops_timing_tags_and_rolling_duplicates() {
        let vtt = "WEBVTT\nKind: captions\nLanguage: en\n\n00:00:00.000 --> 00:00:02.000\n\
                   We&#39;re no<00:00:01.000><c> strangers</c>\n\n\
                   00:00:02.000 --> 00:00:03.000\nWe're no strangers\n\n\
                   2\n00:00:03.000 --> 00:00:04.000\nto love\n";
        assert_eq!(vtt_to_text(vtt), "We're no strangers to love");
    }
}
