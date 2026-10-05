use std::future::Future;
use std::path::PathBuf;

use axum::{
    extract::{Multipart, State},
    Json,
};
use tokio::process::Command;

use crate::encode_plan::plan_encode;
use crate::probe::{probe, MediaInfo, VideoStream};
use crate::proc::{out_time, run_with_progress};
use crate::state::{AppState, Job, JobKind};
use crate::upload::{self, bad, multipart_error, ApiError, JobDir};

const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp", "bmp", "tiff", "tif"];

const MB: f64 = 1024.0 * 1024.0;

/// Leave a little headroom under the requested size so container overhead
/// doesn't push the result past the target.
const SIZE_MARGIN: f64 = 0.97;

/// Below this a re-encode stops being worth delivering, so it is where the
/// search gives up rather than a number to creep past.
const MIN_VIDEO_BPS: f64 = 40_000.0;

/// How many times pass 2 may run. The first attempt is the common case; the
/// retries exist for sources where x264 overshoots its requested bitrate.
const MAX_PASS2_ATTEMPTS: usize = 3;

/// AAC is transparent for nearly everyone well before this; a roomier budget buys nothing
/// audible, only a bigger file.
const MAX_AUDIO_BPS: f64 = 256_000.0;
/// Below this even speech falls apart.
const MIN_AUDIO_BPS: f64 = 24_000.0;
/// Under this, one good channel beats two starved ones.
const MONO_BELOW_BPS: f64 = 64_000.0;
const MAX_AUDIO_ATTEMPTS: usize = 3;

/// POST /api/compress — multipart upload with a `target_mb` field and a
/// `file`. Videos are re-encoded with a bitrate computed to land under the
/// target (two-pass x264); audio gets an AAC bitrate solved the same way;
/// images are searched down in quality/scale.
pub async fn start_compress(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<Job>, ApiError> {
    let dir = JobDir::create(&state)?;

    let mut target_mb: Option<f64> = None;
    let mut combine_audio = false;
    let mut upload = None;

    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        match field.name() {
            Some("target_mb") => {
                // Validated on arrival, ahead of the file, like the transcode format.
                let value = upload::text(field)
                    .await?
                    .parse::<f64>()
                    .ok()
                    .filter(|v| (0.1..=100_000.0).contains(v))
                    .ok_or_else(|| bad("target_mb must be a number between 0.1 and 100000"))?;
                target_mb = Some(value);
            }
            Some("combine_audio") => combine_audio = upload::truthy(&upload::text(field).await?),
            Some("file") => upload = Some(upload::save_file(field, dir.path()).await?),
            _ => {}
        }
    }

    let target_mb = target_mb.ok_or_else(|| bad("missing 'target_mb' field"))?;
    let upload = upload.ok_or_else(|| bad("missing 'file' upload"))?;

    let target_bytes = (target_mb * MB) as u64;
    if upload.size <= target_bytes {
        return Err(bad(format!(
            "'{}' is already {:.1} MB, at or under the {target_mb} MB target",
            upload.original_name,
            upload.size as f64 / MB
        )));
    }

    let is_image = IMAGE_EXTS.contains(&upload.ext.as_str());
    let mut detail = format!(
        "{:.1} MB → ≤ {} MB",
        upload.size as f64 / MB,
        trim_float(target_mb)
    );
    if combine_audio && !is_image {
        detail.push_str(" · merge audio");
    }

    let (id, dir) = dir.keep();
    let job = Job::new(
        id.clone(),
        JobKind::Compress,
        upload.original_name.clone(),
        detail,
        dir.clone(),
    );
    state.insert_job(job.clone());

    let ctx = Ctx {
        state: state.clone(),
        id: id.clone(),
        source: upload.path,
        dir,
        stem: upload::stem(&upload.original_name),
        target_bytes,
        target_mb,
    };
    if is_image {
        let webp = upload.ext == "webp";
        state.spawn_job(&id, compress_image(ctx, webp));
    } else {
        state.spawn_job(&id, compress_media(ctx, combine_audio));
    }

    Ok(Json(job))
}

/// What every compression path needs to know about its job.
struct Ctx {
    state: AppState,
    id: String,
    source: PathBuf,
    dir: PathBuf,
    stem: String,
    target_bytes: u64,
    /// The target as the user typed it, for messages.
    target_mb: f64,
}

impl Ctx {
    /// Where the result goes on disk, and the name it is offered under. The two are separate so
    /// that no part of the client's filename becomes part of a path.
    fn output(&self, ext: &str) -> (PathBuf, String) {
        (
            self.dir.join(format!("output.{ext}")),
            format!("{}.compressed.{ext}", self.stem),
        )
    }

    fn fail(&self, msg: impl Into<String>) {
        self.state.fail(&self.id, msg);
    }

    fn progress(&self, pct: f32) {
        self.state.set_progress(&self.id, pct);
    }

    async fn finish(&self, output: PathBuf, output_name: String) {
        let _ = tokio::fs::remove_file(&self.source).await;
        self.state.complete(&self.id, output, output_name);
    }
}

/// Anything that is not a still image: probe it, then compress as video or as audio.
async fn compress_media(ctx: Ctx, combine_audio: bool) {
    let _slot = ctx.state.acquire_slot(JobKind::Compress).await;
    ctx.state.set_running(&ctx.id);

    let info = probe(&ctx.source).await;
    let Some((info, duration)) = info.and_then(|i| i.duration.map(|d| (i, d))) else {
        return ctx.fail("could not read media duration — is this a valid video or audio file?");
    };

    match &info.video {
        Some(video) => compress_video(&ctx, &info, video, duration, combine_audio).await,
        // The upload form has always accepted audio, but this path used to run x264 on it and
        // fail with ffmpeg's "Output file does not contain any stream".
        None if !info.audio.is_empty() => {
            compress_audio(&ctx, &info, duration, combine_audio).await
        }
        None => ctx.fail("this file has no video or audio stream to compress"),
    }
}

/// `[0:a:0][0:a:1]...amix` over every audio stream, labelled `[aout]`.
fn mix_filter(streams: usize) -> String {
    let labels: String = (0..streams).map(|i| format!("[0:a:{i}]")).collect();
    format!("{labels}amix=inputs={streams}:duration=longest:normalize=0[aout]")
}

/// Two-pass x264 encode at a bitrate computed from the clip duration so the
/// result lands just under the byte target.
async fn compress_video(
    ctx: &Ctx,
    info: &MediaInfo,
    video: &VideoStream,
    duration: f64,
    combine_audio: bool,
) {
    // When asked, mix every audio stream into one track (e.g. ShadowPlay's
    // separate game/mic tracks). Only meaningful with 2+ audio streams.
    let audio_streams = info.audio.len();
    let audio_filter = (combine_audio && audio_streams >= 2).then(|| mix_filter(audio_streams));

    let total_bps = (ctx.target_bytes as f64 * 8.0 * SIZE_MARGIN) / duration;
    // Give audio a slice of the budget appropriate to how tight it is.
    let audio_bps: f64 = if audio_streams == 0 {
        0.0
    } else if total_bps > 1_000_000.0 {
        128_000.0
    } else if total_bps > 400_000.0 {
        96_000.0
    } else {
        64_000.0
    };
    let video_bps = (total_bps - audio_bps).max(MIN_VIDEO_BPS);
    let audio_kbps = format!("{}k", (audio_bps.max(64_000.0) / 1000.0) as u64);

    // Decide what to encode before deciding how hard to work at it. Handing ffmpeg a 1440p60
    // source when the budget only stretches to 0.007 bits per pixel is both the slowest and the
    // ugliest option available; see encode_plan.rs for the measurements.
    let plan = plan_encode(video_bps, video.height, video.fps);
    let vf = plan.video_filter();
    if let Some(ref f) = vf {
        let note = f.replace("scale=-2:", "").replace(",fps=", "p @ ");
        ctx.state.append_detail(&ctx.id, &note);
    }

    // Name the streams rather than leave it to ffmpeg's default pick, which can choose a
    // high-resolution cover image over the actual video.
    let video_map = format!("0:{}", video.index);
    let (output, output_name) = ctx.output("mp4");
    let passlog = ctx.dir.join("ffmpeg2pass");
    let passlog_arg = passlog.to_string_lossy().to_string();
    let clear_passlog = || {
        for suffix in ["-0.log", "-0.log.mbtree"] {
            let _ = std::fs::remove_file(ctx.dir.join(format!("ffmpeg2pass{suffix}")));
        }
    };

    // Pass 1: analysis only, no audio, output discarded.
    let mut pass1 = Command::new("ffmpeg");
    pass1.arg("-y").arg("-i").arg(&ctx.source);
    pass1.args(["-map", &video_map]);
    // Both passes must see the same frames, or pass 2's statistics describe a different video.
    if let Some(ref f) = vf {
        pass1.args(["-vf", f]);
    }
    let video_kbps = format!("{}k", (video_bps / 1000.0) as u64);
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

    let st = ctx.state.clone();
    let jid = ctx.id.clone();
    let r1 = run_with_progress(&mut pass1, move |line| {
        if let Some(cur) = out_time(line) {
            st.set_progress(&jid, ((cur / duration) * 40.0).min(40.0) as f32);
        }
    })
    .await;
    match r1 {
        Ok(r) if r.success => {}
        Ok(r) => {
            clear_passlog();
            return ctx.fail(r.error_or("ffmpeg pass 1 failed"));
        }
        Err(e) => {
            clear_passlog();
            return ctx.fail(format!("failed to run ffmpeg: {e}"));
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

    let base_detail = ctx
        .state
        .get_job(&ctx.id)
        .map(|j| j.detail)
        .unwrap_or_default();
    let mut attempt_bps = video_bps;
    let mut closest: Option<u64> = None;

    for (attempt, (band_lo, band_hi)) in BANDS.iter().copied().enumerate() {
        let video_kbps = format!("{}k", (attempt_bps / 1000.0) as u64);

        let mut pass2 = Command::new("ffmpeg");
        pass2.arg("-y").arg("-i").arg(&ctx.source);
        // -vf and -filter_complex cannot both be given, so when the audio streams are being mixed
        // the scaling has to move into the complex graph alongside them.
        match (&vf, &audio_filter) {
            (_, Some(filter)) => {
                // Map the original video plus the single mixed-down audio track.
                let graph = match &vf {
                    Some(v) => format!("[{video_map}]{v}[vout];{filter}"),
                    None => filter.clone(),
                };
                let vmap = if vf.is_some() { "[vout]" } else { &video_map };
                pass2
                    .args(["-filter_complex", &graph])
                    .args(["-map", vmap, "-map", "[aout]"]);
            }
            (Some(v), None) => {
                pass2
                    .args(["-map", &video_map, "-map", "0:a:0?"])
                    .args(["-vf", v]);
            }
            (None, None) => {
                pass2.args(["-map", &video_map, "-map", "0:a:0?"]);
            }
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
            .args(["-pass", "2", "-passlogfile", &passlog_arg])
            .args(["-c:a", "aac", "-b:a", &audio_kbps])
            .args(["-pix_fmt", "yuv420p", "-movflags", "+faststart"])
            .args(["-progress", "pipe:1", "-nostats"])
            .arg(&output);

        let st = ctx.state.clone();
        let jid = ctx.id.clone();
        let span = band_hi - band_lo;
        let r2 = run_with_progress(&mut pass2, move |line| {
            if let Some(cur) = out_time(line) {
                let pct = (band_lo + (cur / duration) as f32 * span).min(band_hi);
                st.set_progress(&jid, pct);
            }
        })
        .await;

        match r2 {
            Ok(r) if r.success && output.is_file() => {}
            Ok(r) => {
                clear_passlog();
                return ctx.fail(r.error_or("ffmpeg pass 2 failed"));
            }
            Err(e) => {
                clear_passlog();
                return ctx.fail(format!("failed to run ffmpeg: {e}"));
            }
        }

        let size = file_size(&output).await;
        if size <= ctx.target_bytes {
            clear_passlog();
            return ctx.finish(output, output_name).await;
        }
        closest = Some(closest.map_or(size, |best: u64| best.min(size)));

        let next_bps = retry_video_bps(attempt_bps, duration, size, ctx.target_bytes);
        // At the floor there is nothing left to give up, so stop rather than
        // spend another whole encode producing the same file.
        if next_bps >= attempt_bps || next_bps <= MIN_VIDEO_BPS {
            break;
        }
        attempt_bps = next_bps;
        ctx.state.update_job(&ctx.id, |j| {
            j.detail = format!("{base_detail} · retry {}", attempt + 1)
        });
    }

    clear_passlog();
    ctx.fail(too_big(closest, ctx.target_mb));
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

/// AAC at a bitrate solved from the duration, measured and repeated if it lands over.
async fn compress_audio(ctx: &Ctx, info: &MediaInfo, duration: f64, combine_audio: bool) {
    let mut bps = audio_bps_for(ctx.target_bytes, duration);
    if bps < MIN_AUDIO_BPS {
        let needed = MIN_AUDIO_BPS * duration / 8.0 / SIZE_MARGIN / MB;
        return ctx.fail(format!(
            "{} MB is too small for {} of audio — it needs at least {needed:.1} MB",
            trim_float(ctx.target_mb),
            clock(duration)
        ));
    }

    let mix = (combine_audio && info.audio.len() >= 2).then(|| mix_filter(info.audio.len()));
    let (output, output_name) = ctx.output("m4a");
    let base_detail = ctx
        .state
        .get_job(&ctx.id)
        .map(|j| j.detail)
        .unwrap_or_default();
    let mut closest: Option<u64> = None;
    const BANDS: [(f32, f32); MAX_AUDIO_ATTEMPTS] = [(0.0, 90.0), (90.0, 95.0), (95.0, 99.0)];

    for (attempt, (band_lo, band_hi)) in BANDS.iter().copied().enumerate() {
        let mono = bps < MONO_BELOW_BPS;
        let kbps = (bps / 1000.0) as u64;
        ctx.state.update_job(&ctx.id, |j| {
            j.detail = format!(
                "{base_detail} · AAC {kbps} kbps{}{}",
                if mono { " mono" } else { "" },
                if attempt > 0 {
                    format!(" · retry {attempt}")
                } else {
                    String::new()
                }
            )
        });

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y").arg("-i").arg(&ctx.source);
        match &mix {
            Some(filter) => cmd.args(["-filter_complex", filter, "-map", "[aout]"]),
            None => cmd.args(["-map", "0:a:0"]),
        };
        if mono {
            cmd.args(["-ac", "1"]);
        }
        cmd.args(["-vn", "-c:a", "aac", "-b:a", &format!("{kbps}k")])
            .args(["-movflags", "+faststart", "-progress", "pipe:1", "-nostats"])
            .arg(&output);

        let st = ctx.state.clone();
        let jid = ctx.id.clone();
        let span = band_hi - band_lo;
        let result = run_with_progress(&mut cmd, move |line| {
            if let Some(cur) = out_time(line) {
                st.set_progress(
                    &jid,
                    (band_lo + (cur / duration) as f32 * span).min(band_hi),
                );
            }
        })
        .await;
        match result {
            Ok(r) if r.success && output.is_file() => {}
            Ok(r) => return ctx.fail(r.error_or("ffmpeg could not encode the audio")),
            Err(e) => return ctx.fail(format!("failed to run ffmpeg: {e}")),
        }

        let size = file_size(&output).await;
        if size <= ctx.target_bytes {
            return ctx.finish(output, output_name).await;
        }
        closest = Some(closest.map_or(size, |best: u64| best.min(size)));
        // Audio has almost no fixed overhead, so a straight ratio is a good correction.
        bps *= ctx.target_bytes as f64 * SIZE_MARGIN / size as f64;
        if bps < MIN_AUDIO_BPS {
            break;
        }
    }

    ctx.fail(too_big(closest, ctx.target_mb));
}

/// The audio bitrate a byte budget allows over `duration` seconds, capped where more stops
/// being audible.
fn audio_bps_for(target_bytes: u64, duration: f64) -> f64 {
    if duration <= 0.0 {
        return 0.0;
    }
    (target_bytes as f64 * 8.0 * SIZE_MARGIN / duration).min(MAX_AUDIO_BPS)
}

/// JPEG `-q:v`, best (2) to worst (31). Finer steps than before: the search is a binary search
/// now, so a longer ladder costs one extra encode, not seven.
const JPEG_QUALITIES: &[u32] = &[2, 3, 4, 5, 7, 9, 12, 15, 20, 25, 31];
/// The worst JPEG quality worth keeping before shrinking the picture instead (q9).
const JPEG_FLOOR: usize = 5;
/// WebP `-quality`, best (95) to worst (10).
const WEBP_QUALITIES: &[u32] = &[95, 90, 85, 80, 75, 70, 60, 50, 40, 30, 20, 10];
/// WebP degrades more gracefully, so its floor sits lower (quality 30).
const WEBP_FLOOR: usize = 9;
/// Resolution steps, largest first.
const SCALES: &[f64] = &[1.0, 0.75, 0.5, 0.35, 0.25];

/// Find the best quality that fits, shrinking the image rather than dropping below a quality
/// floor, and only going under the floor once the image is as small as it gets.
async fn compress_image(ctx: Ctx, webp: bool) {
    let _slot = ctx.state.acquire_slot(JobKind::Compress).await;
    ctx.state.set_running(&ctx.id);

    let (qualities, floor, ext) = if webp {
        (WEBP_QUALITIES, WEBP_FLOOR, "webp")
    } else {
        (JPEG_QUALITIES, JPEG_FLOOR, "jpg")
    };
    let (output, output_name) = ctx.output(ext);
    let candidate = ctx.dir.join(format!("candidate.{ext}"));

    let mut attempts = 0u32;
    let found = search_image(qualities.len(), floor, SCALES.len(), |s, q| {
        attempts += 1;
        // A search takes between one and nine encodes; this is an honest-enough bar.
        ctx.progress((attempts as f32 * 10.0).min(95.0));

        let mut cmd = Command::new("ffmpeg");
        cmd.arg("-y").arg("-i").arg(&ctx.source);
        let scale = SCALES[s];
        if scale < 1.0 {
            cmd.args(["-vf", &format!("scale=iw*{scale}:ih*{scale}")]);
        }
        if webp {
            cmd.args(["-quality", &qualities[q].to_string()]);
        } else {
            cmd.args(["-q:v", &qualities[q].to_string()]);
        }
        cmd.arg(&candidate);

        let candidate = candidate.clone();
        let output = output.clone();
        let target = ctx.target_bytes;
        async move {
            let ok = matches!(run_with_progress(&mut cmd, |_| {}).await, Ok(r) if r.success);
            if !ok {
                return Err("ffmpeg could not process this image".to_string());
            }
            if file_size(&candidate).await > target {
                return Ok(false);
            }
            // Every fit the search finds is better than the one before it, so the newest fit
            // is always the one to keep.
            tokio::fs::rename(&candidate, &output)
                .await
                .map_err(|e| format!("could not keep the result: {e}"))?;
            Ok(true)
        }
    })
    .await;
    let _ = tokio::fs::remove_file(&candidate).await;

    match found {
        Ok(Some((s, _))) => {
            if SCALES[s] < 1.0 {
                ctx.state
                    .append_detail(&ctx.id, &format!("scaled to {} %", (SCALES[s] * 100.0)));
            }
            ctx.finish(output, output_name).await;
        }
        Ok(None) => ctx.fail(
            "could not get the image under the target size even at minimum quality — try a larger target",
        ),
        Err(e) => ctx.fail(e),
    }
}

/// Search (scale, quality) indices for the best result that fits. `fits` encodes one candidate
/// and says whether it came in under the target.
///
/// The old search walked every quality at full size, worst included, before trying a smaller
/// picture. That is the same trap encode_plan.rs describes for video: holding on to resolution
/// at all costs. Measured on a 6 MP image, JPEG, scoring each result by SSIM after scaling it
/// back up to full size, with the number of encodes each search needed:
///
/// ```text
///   target   old search                     this search
///   600 KB   q6  full size  0.9941   2      q4 full size  0.9958   5
///   250 KB   q25 full size  0.9778   6      q7 at 75 %    0.9842   6
///   150 KB   q20 at 75 %    0.9715  12      q5 at 50 %    0.9785   7
///    60 KB   q31 at 50 %    0.9549  21      q9 at 35 %    0.9667   7
///    40 KB   q20 at 35 %    0.9547  26      q7 at 25 %    0.9627   9
/// ```
///
/// Over eight targets from 25 KB to 600 KB that is 54 encodes instead of 121, and a better
/// picture at every target but the smallest, where both land on the same answer. WebP degrades
/// more gracefully, so its floor is lower; there it takes 40 encodes instead of 60 and is never
/// more than 0.0005 SSIM behind the old choice.
async fn search_image<F, Fut>(
    qualities: usize,
    floor: usize,
    scales: usize,
    mut fits: F,
) -> Result<Option<(usize, usize)>, String>
where
    F: FnMut(usize, usize) -> Fut,
    Fut: Future<Output = Result<bool, String>>,
{
    debug_assert!(floor > 0 && floor < qualities && scales > 0);

    // Many uploads are lossless PNG or BMP that fit at the best quality straight away.
    if fits(0, 0).await? {
        return Ok(Some((0, 0)));
    }

    for s in 0..scales {
        if fits(s, floor).await? {
            // Something at or above the floor fits here: find the best of it. At full size the
            // best quality is already known to miss.
            let (mut lo, mut hi) = (if s == 0 { 1 } else { 0 }, floor);
            while lo < hi {
                let mid = (lo + hi) / 2;
                if fits(s, mid).await? {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            return Ok(Some((s, hi)));
        }
    }

    // Even the smallest picture misses at the floor quality: go below it, there.
    let s = scales - 1;
    let worst = qualities - 1;
    if worst == floor || !fits(s, worst).await? {
        return Ok(None);
    }
    let (mut lo, mut hi) = (floor + 1, worst);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if fits(s, mid).await? {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Ok(Some((s, hi)))
}

async fn file_size(path: &std::path::Path) -> u64 {
    tokio::fs::metadata(path)
        .await
        .map(|m| m.len())
        .unwrap_or(u64::MAX)
}

fn too_big(closest: Option<u64>, target_mb: f64) -> String {
    let closest_mb = closest.unwrap_or(0) as f64 / MB;
    format!(
        "could not get this under {} MB — the closest was {closest_mb:.1} MB. Try a larger target.",
        trim_float(target_mb)
    )
}

/// "3:07" or "1:02:03".
fn clock(seconds: f64) -> String {
    let s = seconds.round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
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

#[cfg(test)]
mod tests {
    use super::*;

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

    type Pick = Option<(usize, usize)>;

    /// Run the search against a table of sizes instead of ffmpeg. `size(scale, quality)` must
    /// shrink as either index grows, like real encodes do.
    fn search(
        qualities: usize,
        floor: usize,
        scales: usize,
        target: u64,
        size: impl Fn(usize, usize) -> u64,
    ) -> (Pick, Vec<(usize, usize)>) {
        let mut tried = Vec::new();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let found = rt
            .block_on(search_image(qualities, floor, scales, |s, q| {
                tried.push((s, q));
                std::future::ready(Ok(size(s, q) <= target))
            }))
            .unwrap();
        (found, tried)
    }

    /// Bytes for a picture at scale index `s` and quality index `q`: each scale step halves the
    /// pixels, each quality step takes off a fifth.
    fn model(s: usize, q: usize) -> u64 {
        (1_000_000.0 * 0.5f64.powi(s as i32) * 0.8f64.powi(q as i32)) as u64
    }

    #[test]
    fn a_roomy_target_takes_the_best_quality_in_one_encode() {
        let (found, tried) = search(11, 5, 5, 2_000_000, model);
        assert_eq!(found, Some((0, 0)));
        assert_eq!(tried.len(), 1);
    }

    #[test]
    fn it_finds_the_best_quality_that_fits_at_full_size() {
        // 0.8^3 = 0.512 MB fits under 0.6 MB, 0.8^2 = 0.64 MB does not.
        let (found, _) = search(11, 5, 5, 600_000, model);
        assert_eq!(found, Some((0, 3)));
    }

    #[test]
    fn it_shrinks_the_picture_before_going_below_the_floor() {
        // Full size only fits below the floor (0.8^6 = 0.26 MB); half size fits at the floor.
        let (found, tried) = search(11, 5, 5, 280_000, model);
        let (s, q) = found.unwrap();
        assert!(s > 0, "should have scaled down, picked {s}/{q}");
        assert!(
            q <= 5,
            "should not have gone below the floor, picked {s}/{q}"
        );
        assert!(
            !tried.iter().any(|&(_, q)| q > 5),
            "never tried below the floor: {tried:?}"
        );
    }

    #[test]
    fn below_the_floor_only_at_the_smallest_size() {
        // Smallest size at the floor is 1e6 / 16 * 0.8^5 = 20 KB; ask for less.
        let (found, _) = search(11, 5, 5, 15_000, model);
        let (s, q) = found.unwrap();
        assert_eq!(s, 4);
        assert!(q > 5);
        assert!(model(s, q) <= 15_000);
        assert!(
            model(s, q - 1) > 15_000,
            "a better quality would also have fitted"
        );
    }

    #[test]
    fn an_impossible_target_reports_nothing() {
        let (found, tried) = search(11, 5, 5, 10, model);
        assert_eq!(found, None);
        // One look at full size, one at the floor per scale, one at the very worst.
        assert_eq!(tried.len(), 1 + 5 + 1);
    }

    #[test]
    fn the_search_stays_cheap() {
        for target in (1..200).map(|k| k * 5_000) {
            let (_, tried) = search(11, 5, 5, target, model);
            assert!(tried.len() <= 10, "{} encodes for {target}", tried.len());
        }
    }

    #[test]
    fn an_ffmpeg_failure_stops_the_search() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let result = rt.block_on(search_image(11, 5, 5, |_, _| {
            std::future::ready(Err::<bool, _>("broken".to_string()))
        }));
        assert_eq!(result, Err("broken".to_string()));
    }

    #[test]
    fn audio_bitrate_follows_the_budget() {
        // 10 MB over three minutes is ~452 kbps, which is more than audio needs.
        assert_eq!(audio_bps_for((10.0 * MB) as u64, 180.0), MAX_AUDIO_BPS);
        // 2 MB over three minutes leaves ~90 kbps.
        let bps = audio_bps_for((2.0 * MB) as u64, 180.0);
        assert!((85_000.0..95_000.0).contains(&bps), "{bps}");
        assert_eq!(audio_bps_for(1_000, 0.0), 0.0);
    }

    #[test]
    fn durations_read_like_a_player() {
        assert_eq!(clock(187.0), "3:07");
        assert_eq!(clock(3723.4), "1:02:03");
    }
}
