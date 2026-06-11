use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use axum::{extract::State, http::StatusCode, Json};
use chrono::Utc;
use serde::Deserialize;
use tokio::process::Command;
use uuid::Uuid;

use crate::proc::run_with_progress;
use crate::state::{AppState, Job, JobKind, JobStatus};

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
) -> Result<Json<Job>, (StatusCode, String)> {
    let url = req.url.trim().to_string();
    if url.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "url is required".into()));
    }
    let format = req.format.trim().to_lowercase();
    if !is_audio_format(&format) && !VIDEO_FORMATS.contains(&format.as_str()) {
        return Err((StatusCode::BAD_REQUEST, format!("unsupported format: {format}")));
    }
    let audio = is_audio_format(&format);
    let quality = if req.quality.trim().is_empty() {
        "best".to_string()
    } else {
        req.quality.trim().to_string()
    };
    if !audio && !QUALITIES.contains(&quality.as_str()) {
        return Err((StatusCode::BAD_REQUEST, format!("unsupported quality: {quality}")));
    }

    let id = Uuid::new_v4().to_string();
    let dir = state.job_dir(&id);
    if let Err(e) = fs::create_dir_all(&dir) {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("mkdir failed: {e}")));
    }

    let detail = if audio {
        format!("audio · {format}")
    } else {
        let q = if quality == "best" { "best".to_string() } else { format!("{quality}p") };
        format!("{q} · {format}")
    };

    let job = Job {
        id: id.clone(),
        kind: JobKind::Download,
        status: JobStatus::Queued,
        progress: 0.0,
        title: url.clone(),
        detail,
        output_name: None,
        output_size: None,
        error: None,
        created_at: Utc::now().timestamp(),
        output_path: None,
        dir: dir.clone(),
    };
    state.insert_job(job.clone());

    // Run the download in the background and report progress into the job map.
    tokio::spawn(run_download(state, id, url, format, quality, dir));

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
    state.update_job(&id, |j| j.status = JobStatus::Running);

    let out_template = format!("{}/%(title).200B.%(ext)s", dir.display());
    let mut args: Vec<String> = vec![
        "--no-playlist".into(),
        "--no-color".into(),
        "--newline".into(),
        "--no-warnings".into(),
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
        let selector = if quality == "best" {
            "bv*+ba/b".to_string()
        } else {
            format!("bv*[height<={q}]+ba/b[height<={q}]", q = quality)
        };
        args.push("-f".into());
        args.push(selector);
        args.push("--merge-output-format".into());
        args.push(format.clone());
    }
    args.push(url);

    let mut cmd = Command::new("yt-dlp");
    cmd.args(&args);

    let state_for_lines = state.clone();
    let id_for_lines = id.clone();
    let result = run_with_progress(&mut cmd, |line| {
        if let Some(rest) = line.strip_prefix("PG:") {
            if let Some(pct) = parse_percent(rest) {
                state_for_lines.update_job(&id_for_lines, |j| {
                    if pct > j.progress {
                        j.progress = pct;
                    }
                });
            }
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
                let size = fs::metadata(&path).ok().map(|m| m.len());
                state.update_job(&id, |j| {
                    j.status = JobStatus::Completed;
                    j.progress = 100.0;
                    j.output_path = Some(path.clone());
                    j.output_name = Some(name);
                    j.output_size = size;
                });
            }
            None => fail(&state, &id, "download finished but no output file was produced".into()),
        },
        Ok(r) => fail(
            &state,
            &id,
            if r.error_tail.is_empty() {
                "yt-dlp exited with an error".into()
            } else {
                r.error_tail
            },
        ),
        Err(e) => fail(&state, &id, format!("failed to run yt-dlp: {e}")),
    }
}

fn fail(state: &AppState, id: &str, msg: String) {
    state.update_job(id, |j| {
        j.status = JobStatus::Failed;
        j.error = Some(msg);
    });
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
        if best.as_ref().map_or(true, |(t, _)| modified >= *t) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, p)| p)
}
