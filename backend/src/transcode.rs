use std::path::PathBuf;

use axum::{
    extract::{Multipart, State},
    Json,
};
use tokio::process::Command;

use crate::probe::{probe, MediaInfo, VideoStream};
use crate::proc::{out_time, run_with_progress};
use crate::state::{AppState, Job, JobKind};
use crate::upload::{self, bad, multipart_error, ApiError, JobDir};

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
) -> Result<Json<Job>, ApiError> {
    let dir = JobDir::create(&state)?;

    let mut target: Option<String> = None;
    let mut combine_audio = false;
    let mut upload = None;

    while let Some(field) = multipart.next_field().await.map_err(multipart_error)? {
        match field.name() {
            Some("format") => {
                let format = upload::text(field).await?.to_lowercase();
                // Checked as soon as it arrives — the UI sends it ahead of the file — so a bad
                // value is turned away before gigabytes are uploaded for nothing.
                if !TARGET_FORMATS.contains(&format.as_str()) {
                    return Err(bad(format!("unsupported target format: {format}")));
                }
                target = Some(format);
            }
            Some("combine_audio") => combine_audio = upload::truthy(&upload::text(field).await?),
            Some("file") => upload = Some(upload::save_file(field, dir.path()).await?),
            _ => {}
        }
    }

    let target = target.ok_or_else(|| bad("missing 'format' field"))?;
    let upload = upload.ok_or_else(|| bad("missing 'file' upload"))?;

    let output_name = format!("{}.{target}", upload::stem(&upload.original_name));
    let source_ext = if upload.ext.is_empty() {
        "?"
    } else {
        upload.ext.as_str()
    };
    let detail = format!("{source_ext} → {target}");

    let (id, dir) = dir.keep();
    // The on-disk name is fixed; the client's filename only reaches the download's
    // Content-Disposition. A 250-byte upload name plus an extension would otherwise exceed the
    // filesystem's name limit and fail the encode at the very end.
    let output = dir.join(format!("output.{target}"));
    let job = Job::new(
        id.clone(),
        JobKind::Transcode,
        upload.original_name,
        detail,
        dir,
    );
    state.insert_job(job.clone());
    state.spawn_job(
        &id,
        run_transcode(
            state.clone(),
            id.clone(),
            upload.path,
            output,
            output_name,
            target,
            combine_audio,
        ),
    );

    Ok(Json(job))
}

async fn run_transcode(
    state: AppState,
    id: String,
    source: PathBuf,
    output: PathBuf,
    output_name: String,
    target: String,
    combine_audio: bool,
) {
    let _slot = state.acquire_slot(JobKind::Transcode).await;
    state.set_running(&id);

    let info = probe(&source).await;
    let audio_streams = info.as_ref().map_or(0, |i| i.audio.len());
    let is_audio = is_audio_target(&target);
    let mix = combine_audio && audio_streams >= 2;

    if audio_streams >= 2 && !mix && is_audio && !supports_multiple_audio(&target) {
        // Keeping tracks separate is impossible in a single-stream container;
        // fail with guidance instead of silently dropping tracks.
        return state.fail(
            &id,
            format!(
                "the source has {audio_streams} audio tracks but {} files can only hold one — \
                 choose \"Combine into one track\", or convert to MKV, MP4, M4A or OGG to keep them separate",
                target.to_uppercase()
            ),
        );
    }

    let plan = plan_streams(&target, info.as_ref(), mix);
    // When the probe failed, fall back to ffmpeg's own selection rather than dropping the video.
    let video_map = match &info {
        Some(i) => i.video.as_ref().map(|v| format!("0:{}", v.index)),
        None => Some("0:v:0?".to_string()),
    };

    let mut cmd = Command::new("ffmpeg");
    cmd.arg("-y").arg("-i").arg(&source);
    if mix {
        // Mix every source audio stream into a single track.
        let labels: String = (0..audio_streams).map(|i| format!("[0:a:{i}]")).collect();
        cmd.args([
            "-filter_complex",
            &format!("{labels}amix=inputs={audio_streams}:duration=longest:normalize=0[aout]"),
        ]);
        if let (false, Some(v)) = (is_audio, &video_map) {
            cmd.args(["-map", v]);
        }
        cmd.args(["-map", "[aout]"]);
    } else if !is_audio {
        // Video containers can carry multiple audio tracks: keep them all
        // instead of ffmpeg's default single-stream pick.
        if let Some(v) = &video_map {
            cmd.args(["-map", v]);
        }
        cmd.args(["-map", "0:a?"]);
    } else if audio_streams >= 2 {
        // Multi-track-capable audio container: keep every track.
        cmd.args(["-map", "0:a"]);
    }

    if !is_audio {
        if plan.copy_video {
            cmd.args(["-c:v", "copy"]);
            let hevc = info
                .as_ref()
                .and_then(|i| i.video.as_ref())
                .is_some_and(|v| v.codec == "hevc");
            if hevc && matches!(target.as_str(), "mp4" | "mov") {
                // Apple's players only accept HEVC in MP4 under the hvc1 tag.
                cmd.args(["-tag:v", "hvc1"]);
            }
        } else {
            cmd.args(video_encode_args(&target));
        }
    }
    if plan.copy_audio {
        cmd.args(["-c:a", "copy"]);
    } else {
        cmd.args(audio_encode_args(&target));
    }
    cmd.args(container_args(&target));
    cmd.args(["-progress", "pipe:1", "-nostats"]).arg(&output);

    let has_video = info.as_ref().is_none_or(|i| i.video.is_some());
    let copied_everything = plan.copy_audio && (is_audio || !has_video || plan.copy_video);
    if copied_everything {
        state.append_detail(&id, "no re-encode");
    } else if plan.copy_video {
        state.append_detail(&id, "video copied");
    }

    let duration = info.as_ref().and_then(|i| i.duration);
    let st = state.clone();
    let jid = id.clone();
    let result = run_with_progress(&mut cmd, move |line| {
        if let (Some(total), Some(cur)) = (duration, out_time(line)) {
            st.set_progress(&jid, ((cur / total) * 100.0).min(99.0) as f32);
        }
    })
    .await;

    match result {
        Ok(r) if r.success && output.is_file() => {
            // Source no longer needed once the output exists.
            let _ = tokio::fs::remove_file(&source).await;
            state.complete(&id, output, output_name);
        }
        Ok(r) => state.fail(&id, r.error_or("ffmpeg exited with an error")),
        Err(e) => state.fail(&id, format!("failed to run ffmpeg: {e}")),
    }
}

/// Which streams can go into the target untouched.
#[derive(Debug, PartialEq)]
struct StreamPlan {
    copy_video: bool,
    copy_audio: bool,
}

/// Copy a stream whenever the target container takes its codec as it is.
///
/// Re-encoding something that is already H.264 just to change the box around it is all cost and
/// no benefit: a 20 s 1080p H.264/AAC MKV took 15.4 s to re-encode to MP4 and 0.18 s to copy,
/// four cores, and the copy is bit-for-bit the original picture. MKV to MP4 is exactly what
/// recording software produces and people then need, so this is the common case, not a corner.
fn plan_streams(target: &str, info: Option<&MediaInfo>, mixing_audio: bool) -> StreamPlan {
    let Some(info) = info else {
        return StreamPlan {
            copy_video: false,
            copy_audio: false,
        };
    };
    StreamPlan {
        copy_video: info
            .video
            .as_ref()
            .is_some_and(|v| video_copyable(target, v)),
        // A mix is a new signal by definition.
        copy_audio: !mixing_audio && info.audio.iter().all(|a| audio_copyable(target, &a.codec)),
    }
}

fn video_copyable(target: &str, v: &VideoStream) -> bool {
    let codec = v.codec.as_str();
    let pix_fmt = v.pix_fmt.as_deref().unwrap_or("");
    // The re-encode path forces 8-bit 4:2:0 for MP4/MOV because that is what plays everywhere;
    // a copy has to meet the same bar or "convert to MP4" would hand back a file browsers refuse.
    let plays_everywhere = matches!(pix_fmt, "yuv420p" | "yuvj420p");
    match target {
        "mp4" => match codec {
            "h264" => plays_everywhere,
            "hevc" | "av1" => plays_everywhere || pix_fmt == "yuv420p10le",
            _ => false,
        },
        "mov" => match codec {
            "h264" => plays_everywhere,
            "hevc" => plays_everywhere || pix_fmt == "yuv420p10le",
            _ => false,
        },
        "mkv" => matches!(codec, "h264" | "hevc" | "av1" | "vp9" | "vp8"),
        "webm" => matches!(codec, "vp9" | "vp8" | "av1"),
        // AVI is offered for old hardware players, which want the XviD tag the encode sets.
        _ => false,
    }
}

fn audio_copyable(target: &str, codec: &str) -> bool {
    match target {
        "mp4" | "mov" => matches!(codec, "aac" | "mp3" | "alac"),
        "mkv" => matches!(
            codec,
            "aac" | "mp3" | "opus" | "vorbis" | "flac" | "ac3" | "eac3" | "alac"
        ),
        "webm" => matches!(codec, "opus" | "vorbis"),
        "avi" | "mp3" => codec == "mp3",
        "m4a" => matches!(codec, "aac" | "alac"),
        "ogg" => codec == "vorbis",
        "opus" => codec == "opus",
        "wav" => codec == "pcm_s16le",
        "flac" => codec == "flac",
        _ => false,
    }
}

/// Whether the target container is audio-only (single audio stream).
fn is_audio_target(target: &str) -> bool {
    matches!(target, "mp3" | "m4a" | "ogg" | "opus" | "wav" | "flac")
}

/// Audio-only containers that can still hold more than one audio stream.
fn supports_multiple_audio(target: &str) -> bool {
    matches!(target, "m4a" | "ogg")
}

/// ffmpeg arguments to encode the video stream for a target container.
fn video_encode_args(target: &str) -> &'static [&'static str] {
    match target {
        "mp4" | "mov" => &[
            "-c:v", "libx264", "-preset", "medium", "-crf", "23", "-pix_fmt", "yuv420p",
        ],
        "mkv" => &["-c:v", "libx264", "-preset", "medium", "-crf", "23"],
        // libvpx-vp9 defaults to -cpu-used 1, among its slowest settings. At 4 a 6 s 1080p clip
        // took 16.5 s instead of 49.6 s on four cores, for a file 6 % larger and an SSIM of
        // 0.9965 against 0.9964.
        "webm" => &[
            "-c:v",
            "libvpx-vp9",
            "-b:v",
            "0",
            "-crf",
            "32",
            "-row-mt",
            "1",
            "-deadline",
            "good",
            "-cpu-used",
            "4",
        ],
        "avi" => &["-c:v", "mpeg4", "-vtag", "XVID", "-qscale:v", "4"],
        _ => &[],
    }
}

/// ffmpeg arguments to encode the audio stream(s) for a target container.
fn audio_encode_args(target: &str) -> &'static [&'static str] {
    match target {
        "mp4" | "mov" | "mkv" | "m4a" => &["-c:a", "aac", "-b:a", "192k"],
        "webm" => &["-c:a", "libopus"],
        "avi" => &["-c:a", "libmp3lame", "-q:a", "4"],
        "mp3" => &["-c:a", "libmp3lame", "-q:a", "2"],
        "ogg" => &["-c:a", "libvorbis", "-q:a", "5"],
        "opus" => &["-c:a", "libopus", "-b:a", "160k"],
        "wav" => &["-c:a", "pcm_s16le"],
        "flac" => &["-c:a", "flac"],
        _ => &[],
    }
}

/// Arguments about the file rather than any one stream.
fn container_args(target: &str) -> &'static [&'static str] {
    match target {
        // Index at the front, so a browser can start playing before the whole file arrives.
        "mp4" => &["-movflags", "+faststart"],
        "mp3" | "m4a" | "ogg" | "opus" | "wav" | "flac" => &["-vn"],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::AudioStream;

    fn source(video: Option<(&str, &str)>, audio: &[&str]) -> MediaInfo {
        MediaInfo {
            duration: Some(10.0),
            video: video.map(|(codec, pix_fmt)| VideoStream {
                index: 0,
                codec: codec.into(),
                height: Some(1080),
                fps: Some(30.0),
                pix_fmt: Some(pix_fmt.into()),
            }),
            audio: audio
                .iter()
                .map(|c| AudioStream { codec: (*c).into() })
                .collect(),
        }
    }

    fn plan(target: &str, info: &MediaInfo) -> (bool, bool) {
        let p = plan_streams(target, Some(info), false);
        (p.copy_video, p.copy_audio)
    }

    #[test]
    fn a_recording_in_mkv_is_remuxed_into_mp4() {
        let obs = source(Some(("h264", "yuv420p")), &["aac", "aac"]);
        assert_eq!(plan("mp4", &obs), (true, true));
        assert_eq!(plan("mov", &obs), (true, true));
    }

    #[test]
    fn only_the_stream_that_does_not_fit_is_re_encoded() {
        let webm_audio = source(Some(("h264", "yuv420p")), &["opus"]);
        assert_eq!(plan("mp4", &webm_audio), (true, false));
        let vp9 = source(Some(("vp9", "yuv420p")), &["opus"]);
        assert_eq!(plan("mp4", &vp9), (false, false));
        assert_eq!(plan("webm", &vp9), (true, true));
        assert_eq!(plan("mkv", &vp9), (true, true));
    }

    #[test]
    fn mp4_keeps_its_plays_everywhere_promise() {
        // 4:4:4 and 10-bit H.264 are valid MP4 but most browsers and phones refuse them.
        let lossless = source(Some(("h264", "yuv444p")), &["aac"]);
        assert_eq!(plan("mp4", &lossless), (false, true));
        assert_eq!(plan("mkv", &lossless), (true, true));
        let ten_bit = source(Some(("h264", "yuv420p10le")), &["aac"]);
        assert_eq!(plan("mp4", &ten_bit), (false, true));
        let hevc_ten_bit = source(Some(("hevc", "yuv420p10le")), &["aac"]);
        assert_eq!(plan("mp4", &hevc_ten_bit), (true, true));
    }

    #[test]
    fn avi_is_always_encoded_for_old_players() {
        let mpeg4 = source(Some(("mpeg4", "yuv420p")), &["mp3"]);
        assert_eq!(plan("avi", &mpeg4), (false, true));
    }

    #[test]
    fn audio_targets_copy_only_their_own_codec() {
        let song = source(None, &["aac"]);
        assert!(plan("m4a", &song).1);
        assert!(!plan("mp3", &song).1);
        assert!(plan("mp3", &source(None, &["mp3"])).1);
        assert!(!plan("ogg", &source(None, &["opus"])).1);
        assert!(plan("wav", &source(None, &["pcm_s16le"])).1);
        assert!(!plan("wav", &source(None, &["pcm_s24le"])).1);
    }

    #[test]
    fn mixing_always_encodes_the_audio() {
        let obs = source(Some(("h264", "yuv420p")), &["aac", "aac"]);
        let p = plan_streams("mp4", Some(&obs), true);
        assert_eq!(
            p,
            StreamPlan {
                copy_video: true,
                copy_audio: false
            }
        );
    }

    #[test]
    fn an_unreadable_source_is_encoded() {
        assert_eq!(
            plan_streams("mp4", None, false),
            StreamPlan {
                copy_video: false,
                copy_audio: false
            }
        );
    }

    #[test]
    fn every_target_has_encode_arguments() {
        for t in TARGET_FORMATS {
            assert!(!audio_encode_args(t).is_empty(), "{t} has no audio args");
            if !is_audio_target(t) {
                assert!(!video_encode_args(t).is_empty(), "{t} has no video args");
            }
        }
    }
}
