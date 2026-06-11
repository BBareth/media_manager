use std::fs;
use std::path::{Path, PathBuf};

use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use uuid::Uuid;

use crate::proc::run_with_progress;
use crate::state::{AppState, Job, JobKind, JobStatus};

const TARGET_FORMATS: &[&str] = &[
    // video containers
    "mp4", "mkv", "mov", "webm", "avi", // audio
    "mp3", "m4a", "ogg", "opus", "wav", "flac",
];

/// POST /api/transcode — multipart upload of a source file plus a `format`
/// field. Streams the upload to disk, then runs ffmpeg in the background.
pub async fn start_transcode(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<Job>, (StatusCode, String)> {
    let id = Uuid::new_v4().to_string();
    let dir = state.job_dir(&id);
    if let Err(e) = fs::create_dir_all(&dir) {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("mkdir failed: {e}")));
    }

    let mut target_format: Option<String> = None;
    let mut source_path: Option<PathBuf> = None;
    let mut original_name: Option<String> = None;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| bad(format!("invalid multipart body: {e}")))?
    {
        match field.name() {
            Some("format") => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| bad(format!("could not read format field: {e}")))?;
                target_format = Some(text.trim().to_lowercase());
            }
            Some("file") => {
                let fname = field
                    .file_name()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "upload".to_string());
                original_name = Some(fname.clone());
                let ext = Path::new(&fname)
                    .extension()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_else(|| "bin".to_string());
                let src = dir.join(format!("source.{ext}"));
                let mut file = tokio::fs::File::create(&src)
                    .await
                    .map_err(|e| internal(format!("cannot create upload file: {e}")))?;
                while let Some(chunk) = field
                    .chunk()
                    .await
                    .map_err(|e| internal(format!("upload stream error: {e}")))?
                {
                    file.write_all(&chunk)
                        .await
                        .map_err(|e| internal(format!("write error: {e}")))?;
                }
                file.flush()
                    .await
                    .map_err(|e| internal(format!("flush error: {e}")))?;
                source_path = Some(src);
            }
            _ => {}
        }
    }

    let target_format = target_format.ok_or_else(|| bad("missing 'format' field".into()))?;
    if !TARGET_FORMATS.contains(&target_format.as_str()) {
        let _ = fs::remove_dir_all(&dir);
        return Err(bad(format!("unsupported target format: {target_format}")));
    }
    let source_path = source_path.ok_or_else(|| bad("missing 'file' upload".into()))?;
    let original_name = original_name.unwrap_or_else(|| "upload".to_string());

    let stem = Path::new(&original_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());
    let output_name = format!("{stem}.{target_format}");
    let output_path = dir.join(&output_name);

    let source_ext = Path::new(&original_name)
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_else(|| "?".to_string());

    let job = Job {
        id: id.clone(),
        kind: JobKind::Transcode,
        status: JobStatus::Queued,
        progress: 0.0,
        title: original_name,
        detail: format!("{source_ext} → {target_format}"),
        output_name: None,
        output_size: None,
        error: None,
        created_at: Utc::now().timestamp(),
        output_path: None,
        dir,
    };
    state.insert_job(job.clone());

    tokio::spawn(run_transcode(
        state,
        id,
        source_path,
        output_path,
        output_name,
        target_format,
    ));

    Ok(Json(job))
}

async fn run_transcode(
    state: AppState,
    id: String,
    source: PathBuf,
    output: PathBuf,
    output_name: String,
    target_format: String,
) {
    state.update_job(&id, |j| j.status = JobStatus::Running);

    let duration = probe_duration(&source).await;

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y").arg("-i").arg(&source);
    for a in ffmpeg_codec_args(&target_format) {
        cmd.arg(a);
    }
    cmd.arg("-progress").arg("pipe:1").arg("-nostats").arg(&output);

    let state_for_lines = state.clone();
    let id_for_lines = id.clone();
    let result = run_with_progress(&mut cmd, |line| {
        if let Some(value) = line.strip_prefix("out_time=") {
            if let (Some(total), Some(cur)) = (duration, parse_ffmpeg_time(value)) {
                if total > 0.0 {
                    let pct = ((cur / total) * 100.0).clamp(0.0, 99.0) as f32;
                    state_for_lines.update_job(&id_for_lines, |j| {
                        if pct > j.progress {
                            j.progress = pct;
                        }
                    });
                }
            }
        }
    })
    .await;

    match result {
        Ok(r) if r.success && output.is_file() => {
            let size = fs::metadata(&output).ok().map(|m| m.len());
            // Source no longer needed once the output exists.
            let _ = fs::remove_file(&source);
            state.update_job(&id, |j| {
                j.status = JobStatus::Completed;
                j.progress = 100.0;
                j.output_path = Some(output.clone());
                j.output_name = Some(output_name.clone());
                j.output_size = size;
            });
        }
        Ok(r) => state.update_job(&id, |j| {
            j.status = JobStatus::Failed;
            j.error = Some(if r.error_tail.is_empty() {
                "ffmpeg exited with an error".into()
            } else {
                r.error_tail
            });
        }),
        Err(e) => state.update_job(&id, |j| {
            j.status = JobStatus::Failed;
            j.error = Some(format!("failed to run ffmpeg: {e}"));
        }),
    }
}

pub async fn probe_duration(path: &Path) -> Option<f64> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()
}

/// Parse ffmpeg's `out_time` value ("HH:MM:SS.microseconds") into seconds.
pub fn parse_ffmpeg_time(s: &str) -> Option<f64> {
    let s = s.trim();
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f64 = parts[0].parse().ok()?;
    let m: f64 = parts[1].parse().ok()?;
    let sec: f64 = parts[2].parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + sec)
}

/// ffmpeg encoding arguments for a given target container/codec.
fn ffmpeg_codec_args(target: &str) -> Vec<&'static str> {
    match target {
        "mp4" => vec![
            "-c:v", "libx264", "-preset", "medium", "-crf", "23", "-pix_fmt", "yuv420p", "-c:a",
            "aac", "-b:a", "192k", "-movflags", "+faststart",
        ],
        "mkv" => vec![
            "-c:v", "libx264", "-preset", "medium", "-crf", "23", "-c:a", "aac", "-b:a", "192k",
        ],
        "mov" => vec![
            "-c:v", "libx264", "-preset", "medium", "-crf", "23", "-pix_fmt", "yuv420p", "-c:a",
            "aac", "-b:a", "192k",
        ],
        "webm" => vec![
            "-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "32", "-row-mt", "1", "-c:a", "libopus",
        ],
        "avi" => vec![
            "-c:v", "mpeg4", "-vtag", "XVID", "-qscale:v", "4", "-c:a", "libmp3lame", "-q:a", "4",
        ],
        "mp3" => vec!["-vn", "-c:a", "libmp3lame", "-q:a", "2"],
        "m4a" => vec!["-vn", "-c:a", "aac", "-b:a", "192k"],
        "ogg" => vec!["-vn", "-c:a", "libvorbis", "-q:a", "5"],
        "opus" => vec!["-vn", "-c:a", "libopus", "-b:a", "160k"],
        "wav" => vec!["-vn", "-c:a", "pcm_s16le"],
        "flac" => vec!["-vn", "-c:a", "flac"],
        _ => vec![],
    }
}

fn bad(msg: String) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, msg)
}

fn internal(msg: String) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, msg)
}
