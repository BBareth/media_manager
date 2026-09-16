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
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("mkdir failed: {e}"),
        ));
    }

    let mut target_format: Option<String> = None;
    let mut source_path: Option<PathBuf> = None;
    let mut original_name: Option<String> = None;
    let mut combine_audio = false;

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
            Some("combine_audio") => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| bad(format!("could not read combine_audio field: {e}")))?;
                combine_audio = text.trim().eq_ignore_ascii_case("true");
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
        combine_audio,
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
    combine_audio: bool,
) {
    state.update_job(&id, |j| j.status = JobStatus::Running);

    let duration = probe_duration(&source).await;
    let audio_streams = probe_audio_streams(&source).await;
    let is_audio = is_audio_target(&target_format);

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y").arg("-i").arg(&source);
    if audio_streams >= 2 && combine_audio {
        // Mix every source audio stream into a single track.
        let labels: String = (0..audio_streams).map(|i| format!("[0:a:{i}]")).collect();
        cmd.args([
            "-filter_complex",
            &format!("{labels}amix=inputs={audio_streams}:duration=longest:normalize=0[aout]"),
        ]);
        if is_audio {
            cmd.args(["-map", "[aout]"]);
        } else {
            cmd.args(["-map", "0:v:0?", "-map", "[aout]"]);
        }
    } else if audio_streams >= 2 && is_audio && !supports_multiple_audio(&target_format) {
        // Keeping tracks separate is impossible in a single-stream container;
        // fail with guidance instead of silently dropping tracks.
        state.update_job(&id, |j| {
            j.status = JobStatus::Failed;
            j.error = Some(format!(
                "the source has {audio_streams} audio tracks but {} files can only hold one — \
                 choose \"Combine into one track\", or convert to MKV, MP4, M4A or OGG to keep them separate",
                target_format.to_uppercase()
            ));
        });
        return;
    } else if !is_audio {
        // Video containers can carry multiple audio tracks: keep them all
        // instead of ffmpeg's default single-stream pick.
        cmd.args(["-map", "0:v:0?", "-map", "0:a?"]);
    } else if audio_streams >= 2 {
        // Multi-track-capable audio container: keep every track.
        cmd.args(["-map", "0:a"]);
    }
    for a in ffmpeg_codec_args(&target_format) {
        cmd.arg(a);
    }
    cmd.arg("-progress")
        .arg("pipe:1")
        .arg("-nostats")
        .arg(&output);

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

/// Count the audio streams in a file via ffprobe.
pub async fn probe_audio_streams(path: &Path) -> usize {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "a",
            "-show_entries",
            "stream=index",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .await;
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .count(),
        _ => 0,
    }
}

/// Height and frame rate of the first video stream, for sizing an encode.
///
/// `r_frame_rate` comes back as a rational like `60/1` (and `0/0` for streams that have no
/// meaningful rate, such as an attached cover image), so it has to be divided rather than parsed
/// as a float. Anything unreadable returns None, and the caller leaves the source alone rather
/// than guessing at it.
pub async fn probe_video_shape(path: &Path) -> (Option<u32>, Option<f64>) {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=height,r_frame_rate",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .await;
    let Ok(o) = output else { return (None, None) };
    if !o.status.success() {
        return (None, None);
    }
    let text = String::from_utf8_lossy(&o.stdout);
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let height = lines
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|h| *h > 0);
    let fps = lines.next().and_then(parse_rational).filter(|f| *f > 0.0);
    (height, fps)
}

/// "60/1" -> 60.0, "30000/1001" -> 29.97, "0/0" -> None.
pub fn parse_rational(s: &str) -> Option<f64> {
    let (num, den) = s.trim().split_once('/')?;
    let num: f64 = num.parse().ok()?;
    let den: f64 = den.parse().ok()?;
    if den == 0.0 {
        None
    } else {
        Some(num / den)
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

/// Whether the target container is audio-only (single audio stream).
fn is_audio_target(target: &str) -> bool {
    matches!(target, "mp3" | "m4a" | "ogg" | "opus" | "wav" | "flac")
}

/// Audio-only containers that can still hold more than one audio stream.
fn supports_multiple_audio(target: &str) -> bool {
    matches!(target, "m4a" | "ogg")
}

/// ffmpeg encoding arguments for a given target container/codec.
fn ffmpeg_codec_args(target: &str) -> Vec<&'static str> {
    match target {
        "mp4" => vec![
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "23",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-movflags",
            "+faststart",
        ],
        "mkv" => vec![
            "-c:v", "libx264", "-preset", "medium", "-crf", "23", "-c:a", "aac", "-b:a", "192k",
        ],
        "mov" => vec![
            "-c:v", "libx264", "-preset", "medium", "-crf", "23", "-pix_fmt", "yuv420p", "-c:a",
            "aac", "-b:a", "192k",
        ],
        "webm" => vec![
            "-c:v",
            "libvpx-vp9",
            "-b:v",
            "0",
            "-crf",
            "32",
            "-row-mt",
            "1",
            "-c:a",
            "libopus",
        ],
        "avi" => vec![
            "-c:v",
            "mpeg4",
            "-vtag",
            "XVID",
            "-qscale:v",
            "4",
            "-c:a",
            "libmp3lame",
            "-q:a",
            "4",
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
