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

use crate::encode_plan::plan_encode;
use crate::proc::run_with_progress;
use crate::state::{AppState, Job, JobKind, JobStatus};
use crate::transcode::{parse_ffmpeg_time, probe_audio_streams, probe_duration, probe_video_shape};

const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "tiff", "tif"];

/// Leave a little headroom under the requested size so container overhead
/// doesn't push the result past the target.
const SIZE_MARGIN: f64 = 0.97;

/// Below this a re-encode stops being worth delivering, so it is where the
/// search gives up rather than a number to creep past.
const MIN_VIDEO_BPS: f64 = 40_000.0;

/// How many times pass 2 may run. The first attempt is the common case; the
/// retries exist for sources where x264 overshoots its requested bitrate.
const MAX_PASS2_ATTEMPTS: usize = 3;

/// POST /api/compress — multipart upload with a `target_mb` field and a
/// `file`. Videos are re-encoded with a bitrate computed to land under the
/// target (two-pass x264); images are searched down in quality/scale.
pub async fn start_compress(
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

    let mut target_mb: Option<f64> = None;
    let mut source_path: Option<PathBuf> = None;
    let mut original_name: Option<String> = None;
    let mut upload_size: u64 = 0;
    let mut combine_audio = false;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| bad(format!("invalid multipart body: {e}")))?
    {
        match field.name() {
            Some("target_mb") => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| bad(format!("could not read target_mb: {e}")))?;
                target_mb = text.trim().parse::<f64>().ok();
            }
            Some("combine_audio") => {
                let text = field.text().await.unwrap_or_default();
                combine_audio = matches!(text.trim(), "true" | "1" | "on" | "yes");
            }
            Some("file") => {
                let fname = field
                    .file_name()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "upload".to_string());
                original_name = Some(fname.clone());
                let ext = Path::new(&fname)
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
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
                    upload_size += chunk.len() as u64;
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

    let target_mb = match target_mb {
        Some(v) if (0.1..=100_000.0).contains(&v) => v,
        Some(_) => {
            let _ = fs::remove_dir_all(&dir);
            return Err(bad("target_mb must be between 0.1 and 100000".into()));
        }
        None => {
            let _ = fs::remove_dir_all(&dir);
            return Err(bad("missing or invalid 'target_mb' field".into()));
        }
    };
    let source_path = source_path.ok_or_else(|| bad("missing 'file' upload".into()))?;
    let original_name = original_name.unwrap_or_else(|| "upload".to_string());

    let target_bytes = (target_mb * 1024.0 * 1024.0) as u64;
    if upload_size <= target_bytes {
        let _ = fs::remove_dir_all(&dir);
        return Err(bad(format!(
            "'{original_name}' is already {:.1} MB, at or under the {target_mb} MB target",
            upload_size as f64 / (1024.0 * 1024.0)
        )));
    }

    let ext = Path::new(&original_name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let is_image = IMAGE_EXTS.contains(&ext.as_str());

    let stem = Path::new(&original_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());
    // Videos always come out as mp4; images keep jpg/webp, everything else
    // becomes jpg (png at reduced quality would change format anyway).
    let out_ext = if is_image {
        if ext == "webp" {
            "webp"
        } else {
            "jpg"
        }
    } else {
        "mp4"
    };
    let output_name = format!("{stem}.compressed.{out_ext}");
    let output_path = dir.join(&output_name);

    let job = Job {
        id: id.clone(),
        kind: JobKind::Compress,
        status: JobStatus::Queued,
        progress: 0.0,
        title: original_name,
        detail: {
            let base = format!(
                "{:.1} MB → ≤ {} MB",
                upload_size as f64 / (1024.0 * 1024.0),
                trim_float(target_mb)
            );
            if combine_audio && !is_image {
                format!("{base} · merge audio")
            } else {
                base
            }
        },
        output_name: None,
        output_size: None,
        error: None,
        created_at: Utc::now().timestamp(),
        output_path: None,
        dir,
    };
    state.insert_job(job.clone());

    if is_image {
        tokio::spawn(compress_image(
            state,
            id,
            source_path,
            output_path,
            output_name,
            target_bytes,
        ));
    } else {
        tokio::spawn(compress_video(
            state,
            id,
            source_path,
            output_path,
            output_name,
            target_bytes,
            combine_audio,
        ));
    }

    Ok(Json(job))
}

/// Two-pass x264 encode at a bitrate computed from the clip duration so the
/// result lands just under the byte target.
async fn compress_video(
    state: AppState,
    id: String,
    source: PathBuf,
    output: PathBuf,
    output_name: String,
    target_bytes: u64,
    combine_audio: bool,
) {
    state.update_job(&id, |j| j.status = JobStatus::Running);

    let Some(duration) = probe_duration(&source).await.filter(|d| *d > 0.0) else {
        return fail(
            &state,
            &id,
            "could not read media duration — is this a valid video file?".into(),
        );
    };

    // When asked, mix every audio stream into one track (e.g. ShadowPlay's
    // separate game/mic tracks). Only meaningful with 2+ audio streams.
    let audio_streams = if combine_audio {
        probe_audio_streams(&source).await
    } else {
        0
    };
    let audio_filter = if audio_streams >= 2 {
        let labels: String = (0..audio_streams).map(|i| format!("[0:a:{i}]")).collect();
        Some(format!(
            "{labels}amix=inputs={audio_streams}:duration=longest:normalize=0[aout]"
        ))
    } else {
        None
    };

    let total_bps = (target_bytes as f64 * 8.0 * SIZE_MARGIN) / duration;
    // Give audio a slice of the budget appropriate to how tight it is.
    let audio_bps: f64 = if total_bps > 1_000_000.0 {
        128_000.0
    } else if total_bps > 400_000.0 {
        96_000.0
    } else {
        64_000.0
    };
    let video_bps = (total_bps - audio_bps).max(MIN_VIDEO_BPS);
    let video_kbps = format!("{}k", (video_bps / 1000.0) as u64);
    let audio_kbps = format!("{}k", (audio_bps / 1000.0) as u64);

    // Decide what to encode before deciding how hard to work at it. Handing ffmpeg a 1440p60
    // source when the budget only stretches to 0.007 bits per pixel is both the slowest and the
    // ugliest option available; see encode_plan.rs for the measurements.
    let (src_h, src_fps) = probe_video_shape(&source).await;
    let plan = plan_encode(video_bps, src_h, src_fps);
    let vf = plan.video_filter();
    if let Some(ref f) = vf {
        let note = f.clone();
        state.update_job(&id, |j| {
            j.detail = format!(
                "{} · {}",
                j.detail,
                note.replace("scale=-2:", "").replace(",fps=", "p @ ")
            )
        });
    }

    let passlog = source.with_file_name("ffmpeg2pass");
    let passlog_arg = passlog.to_string_lossy().to_string();

    // Pass 1: analysis only, no audio, output discarded.
    let mut pass1 = Command::new("ffmpeg");
    pass1.arg("-y").arg("-i").arg(&source);
    // Both passes must see the same frames, or pass 2's statistics describe a different video.
    if let Some(ref f) = vf {
        pass1.args(["-vf", f]);
    }
    pass1
        .args([
            "-c:v",
            "libx264",
            "-preset",
            plan.preset,
            "-b:v",
            &video_kbps,
        ])
        .args(["-pass", "1", "-passlogfile", &passlog_arg])
        .args(["-an", "-f", "mp4", "-progress", "pipe:1", "-nostats"])
        .arg("/dev/null");

    let st = state.clone();
    let jid = id.clone();
    let r1 = run_with_progress(&mut pass1, |line| {
        if let Some(v) = line.strip_prefix("out_time=") {
            if let Some(cur) = parse_ffmpeg_time(v) {
                let pct = ((cur / duration) * 40.0).clamp(0.0, 40.0) as f32;
                st.update_job(&jid, |j| {
                    if pct > j.progress {
                        j.progress = pct
                    }
                });
            }
        }
    })
    .await;
    match r1 {
        Ok(r) if r.success => {}
        Ok(r) => {
            clear_passlog(&source);
            return fail(&state, &id, pick_err(r.error_tail, "ffmpeg pass 1 failed"));
        }
        Err(e) => {
            clear_passlog(&source);
            return fail(&state, &id, format!("failed to run ffmpeg: {e}"));
        }
    }

    // Pass 2, repeated if what comes out is still too big.
    //
    // x264 aims at the bitrate it is given but does not promise to hit it, and
    // on a short clip it has few frames to converge over while the container
    // overhead is proportionally large. SIZE_MARGIN alone does not cover that,
    // and a size target is usually a hard upload limit — a file 4 % over is as
    // rejected as one twice the size. So measure the result and encode again at
    // a corrected bitrate. Only pass 2 repeats: the pass-1 log describes the
    // source rather than the bitrate, which is the whole premise of multi-pass.
    const BANDS: [(f32, f32); MAX_PASS2_ATTEMPTS] = [(40.0, 80.0), (80.0, 90.0), (90.0, 99.0)];

    let base_detail = state.get_job(&id).map(|j| j.detail).unwrap_or_default();
    let mut attempt_bps = video_bps;
    let mut closest: Option<u64> = None;

    for (attempt, (band_lo, band_hi)) in BANDS.iter().copied().enumerate() {
        let video_kbps = format!("{}k", (attempt_bps / 1000.0) as u64);

        let mut pass2 = Command::new("ffmpeg");
        pass2.arg("-y").arg("-i").arg(&source);
        // -vf and -filter_complex cannot both be given, so when the audio streams are being mixed
        // the scaling has to move into the complex graph alongside them.
        if let (Some(v), None) = (&vf, &audio_filter) {
            pass2.args(["-vf", v]);
        }
        pass2
            .args([
                "-c:v",
                "libx264",
                "-preset",
                plan.preset,
                "-b:v",
                &video_kbps,
            ])
            .args(["-pass", "2", "-passlogfile", &passlog_arg]);
        if let Some(ref filter) = audio_filter {
            // Map the original video plus the single mixed-down audio track.
            let graph = match &vf {
                Some(v) => format!("[0:v:0]{v}[vout];{filter}"),
                None => filter.clone(),
            };
            let vmap = if vf.is_some() { "[vout]" } else { "0:v:0" };
            pass2
                .args(["-filter_complex", &graph])
                .args(["-map", vmap, "-map", "[aout]"]);
        }
        pass2
            .args(["-c:a", "aac", "-b:a", &audio_kbps])
            .args(["-pix_fmt", "yuv420p", "-movflags", "+faststart"])
            .args(["-progress", "pipe:1", "-nostats"])
            .arg(&output);

        let st = state.clone();
        let jid = id.clone();
        let span = band_hi - band_lo;
        let r2 = run_with_progress(&mut pass2, move |line| {
            if let Some(v) = line.strip_prefix("out_time=") {
                if let Some(cur) = parse_ffmpeg_time(v) {
                    let pct = (band_lo + (cur / duration) as f32 * span).clamp(band_lo, band_hi);
                    st.update_job(&jid, |j| {
                        if pct > j.progress {
                            j.progress = pct
                        }
                    });
                }
            }
        })
        .await;

        match r2 {
            Ok(r) if r.success && output.is_file() => {}
            Ok(r) => {
                clear_passlog(&source);
                return fail(&state, &id, pick_err(r.error_tail, "ffmpeg pass 2 failed"));
            }
            Err(e) => {
                clear_passlog(&source);
                return fail(&state, &id, format!("failed to run ffmpeg: {e}"));
            }
        }

        let size = fs::metadata(&output).map(|m| m.len()).unwrap_or(u64::MAX);
        if size <= target_bytes {
            clear_passlog(&source);
            let _ = fs::remove_file(&source);
            return finish(&state, &id, output, output_name);
        }
        closest = Some(closest.map_or(size, |best: u64| best.min(size)));

        let next_bps = retry_video_bps(attempt_bps, duration, size, target_bytes);
        // At the floor there is nothing left to give up, so stop rather than
        // spend another whole encode producing the same file.
        if next_bps >= attempt_bps || next_bps <= MIN_VIDEO_BPS {
            break;
        }
        attempt_bps = next_bps;
        state.update_job(&id, |j| {
            j.detail = format!("{base_detail} · retry {}", attempt + 1)
        });
    }

    clear_passlog(&source);
    let closest_mb = closest.unwrap_or(0) as f64 / (1024.0 * 1024.0);
    let target_mb = target_bytes as f64 / (1024.0 * 1024.0);
    fail(
        &state,
        &id,
        format!(
            "could not get this under {target_mb:.1} MB — the closest was {closest_mb:.1} MB. Try a larger target."
        ),
    );
}

/// Bitrate to aim at next when an encode landed over target.
///
/// Correcting by the overall ratio would under-correct every time: the audio
/// track, the container and the moov atom do not shrink when the video bitrate
/// does. Estimate that fixed part from what the last attempt asked for, take it
/// off the target, and solve for the video bitrate that is actually left.
fn retry_video_bps(previous_bps: f64, duration: f64, actual_bytes: u64, target_bytes: u64) -> f64 {
    if duration <= 0.0 || !previous_bps.is_finite() {
        return MIN_VIDEO_BPS;
    }
    let video_bytes = previous_bps * duration / 8.0;
    let fixed_bytes = (actual_bytes as f64 - video_bytes).max(0.0);
    let budget_bytes = target_bytes as f64 * SIZE_MARGIN - fixed_bytes;
    ((budget_bytes * 8.0) / duration).max(MIN_VIDEO_BPS)
}

/// Pass logs are scratch; remove them whatever the outcome.
fn clear_passlog(source: &Path) {
    for suffix in ["-0.log", "-0.log.mbtree"] {
        let _ = fs::remove_file(source.with_file_name(format!("ffmpeg2pass{suffix}")));
    }
}

/// Walk quality down (and resolution if needed) until the image fits.
async fn compress_image(
    state: AppState,
    id: String,
    source: PathBuf,
    output: PathBuf,
    output_name: String,
    target_bytes: u64,
) {
    state.update_job(&id, |j| j.status = JobStatus::Running);

    // For jpg, -q:v runs 2 (best) .. 31 (worst). webp uses -quality 0-100.
    let is_webp = output.extension().map(|e| e == "webp").unwrap_or(false);
    let qualities: &[u32] = if is_webp {
        &[90, 75, 60, 45, 30, 20, 10]
    } else {
        &[3, 6, 10, 15, 20, 25, 31]
    };
    // If even the worst quality is too big, shrink resolution progressively.
    let scales = [1.0_f64, 0.75, 0.5, 0.35, 0.25];

    let total_attempts = (qualities.len() * scales.len()) as f32;
    let mut attempt = 0.0_f32;

    for scale in scales {
        for &q in qualities {
            attempt += 1.0;
            let mut cmd = Command::new("ffmpeg");
            cmd.arg("-y").arg("-i").arg(&source);
            if scale < 1.0 {
                cmd.args(["-vf", &format!("scale=iw*{scale}:ih*{scale}")]);
            }
            if is_webp {
                cmd.args(["-quality", &q.to_string()]);
            } else {
                cmd.args(["-q:v", &q.to_string()]);
            }
            cmd.arg(&output);

            let ok = matches!(run_with_progress(&mut cmd, |_| {}).await, Ok(r) if r.success);
            if !ok {
                return fail(&state, &id, "ffmpeg could not process this image".into());
            }
            let size = fs::metadata(&output).map(|m| m.len()).unwrap_or(u64::MAX);
            state.update_job(&id, |j| {
                j.progress = ((attempt / total_attempts) * 100.0).clamp(0.0, 99.0)
            });
            if size <= target_bytes {
                let _ = fs::remove_file(&source);
                return finish(&state, &id, output, output_name);
            }
        }
    }

    fail(
        &state,
        &id,
        "could not get the image under the target size even at minimum quality — try a larger target".into(),
    )
}

fn finish(state: &AppState, id: &str, output: PathBuf, output_name: String) {
    let size = fs::metadata(&output).ok().map(|m| m.len());
    state.update_job(id, |j| {
        j.status = JobStatus::Completed;
        j.progress = 100.0;
        j.output_path = Some(output.clone());
        j.output_name = Some(output_name.clone());
        j.output_size = size;
    });
}

fn fail(state: &AppState, id: &str, msg: String) {
    state.update_job(id, |j| {
        j.status = JobStatus::Failed;
        j.error = Some(msg);
    });
}

fn pick_err(tail: String, fallback: &str) -> String {
    if tail.is_empty() {
        fallback.to_string()
    } else {
        tail
    }
}

/// "25" instead of "25.0", but keep "2.5".
fn trim_float(v: f64) -> String {
    if (v - v.round()).abs() < f64::EPSILON {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn bad(msg: String) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, msg)
}

fn internal(msg: String) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: f64 = 1024.0 * 1024.0;

    /// The case that prompted this: an 8 s clip asked for 0.5 MB came back at
    /// 0.52 MB. The next attempt has to aim meaningfully lower.
    #[test]
    fn an_overshoot_lowers_the_bitrate() {
        let duration = 8.0;
        let previous = 470_000.0;
        let next = retry_video_bps(previous, duration, 545_880, (0.5 * MB) as u64);
        assert!(next < previous, "{next} should be below {previous}");
        assert!(
            next > MIN_VIDEO_BPS,
            "a 4 % overshoot is not a hopeless one"
        );
    }

    /// Scaling the video bitrate by the overall ratio always under-corrects,
    /// because the audio and the container do not shrink with it. The
    /// correction has to be sharper than that to be worth an extra encode.
    #[test]
    fn it_corrects_harder_than_the_naive_ratio() {
        let duration = 60.0;
        let previous = 1_000_000.0;
        let actual = 12_582_912; // 12 MB
        let target = 10 * 1024 * 1024; // 10 MB

        let naive = previous * (target as f64 / actual as f64);
        let next = retry_video_bps(previous, duration, actual, target);
        assert!(next < naive, "{next} should undercut the naive {naive}");
    }

    /// A target no bitrate can reach lands on the floor, which is the signal
    /// the caller uses to stop retrying.
    #[test]
    fn an_impossible_target_lands_on_the_floor() {
        // 0.1 MB for ten minutes of video, where the audio alone already
        // exceeds the target.
        let next = retry_video_bps(200_000.0, 600.0, 9_000_000, (0.1 * MB) as u64);
        assert_eq!(next, MIN_VIDEO_BPS);
    }

    #[test]
    fn a_zero_duration_cannot_divide_by_zero() {
        assert_eq!(retry_video_bps(500_000.0, 0.0, 1_000, 500), MIN_VIDEO_BPS);
        assert_eq!(retry_video_bps(f64::NAN, 10.0, 1_000, 500), MIN_VIDEO_BPS);
    }

    /// Each attempt must move, or the retry budget is spent re-encoding the
    /// same file at the same bitrate.
    #[test]
    fn successive_attempts_keep_descending() {
        let duration = 30.0;
        let mut bps = 2_000_000.0;
        let target = (5.0 * MB) as u64;
        // Pretend every attempt lands 20 % over.
        for _ in 0..3 {
            let actual = (target as f64 * 1.2) as u64;
            let next = retry_video_bps(bps, duration, actual, target);
            assert!(next < bps, "{next} should be below {bps}");
            bps = next;
        }
    }
}
