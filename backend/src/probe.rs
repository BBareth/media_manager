//! Everything the encoders need to know about a source, from one `ffprobe` call.
//!
//! This used to be three separate ffprobe runs per job (duration, audio stream count, video
//! shape), each re-opening and re-parsing the file. Transcoding now also needs the codecs to
//! decide whether a stream can be copied, which would have made it four.

use std::path::Path;

use serde::Deserialize;
use tokio::process::Command;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MediaInfo {
    /// Seconds.
    pub duration: Option<f64>,
    /// The first real video stream. Cover art embedded in audio files is not one.
    pub video: Option<VideoStream>,
    pub audio: Vec<AudioStream>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VideoStream {
    /// Absolute stream index, for `-map 0:<index>`.
    pub index: u32,
    pub codec: String,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub pix_fmt: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioStream {
    pub codec: String,
}

#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<RawStream>,
    format: Option<RawFormat>,
}

#[derive(Deserialize)]
struct RawStream {
    index: u32,
    codec_type: Option<String>,
    codec_name: Option<String>,
    height: Option<u32>,
    r_frame_rate: Option<String>,
    pix_fmt: Option<String>,
    disposition: Option<Disposition>,
}

#[derive(Deserialize)]
struct Disposition {
    #[serde(default)]
    attached_pic: u8,
}

#[derive(Deserialize)]
struct RawFormat {
    duration: Option<String>,
}

/// Probe a file. `None` means ffprobe could not make sense of it at all.
pub async fn probe(path: &Path) -> Option<MediaInfo> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=index,codec_type,codec_name,height,r_frame_rate,pix_fmt:stream_disposition=attached_pic",
            "-of",
            "json",
        ])
        .arg(path)
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse(&String::from_utf8_lossy(&output.stdout))
}

pub fn parse(json: &str) -> Option<MediaInfo> {
    let probe: Probe = serde_json::from_str(json).ok()?;

    let duration = probe
        .format
        .and_then(|f| f.duration)
        .and_then(|d| d.trim().parse::<f64>().ok())
        .filter(|d| d.is_finite() && *d > 0.0);

    let mut video = None;
    let mut audio = Vec::new();
    for s in probe.streams {
        let codec = s.codec_name.unwrap_or_default();
        match s.codec_type.as_deref() {
            Some("video") if video.is_none() => {
                let cover = s.disposition.is_some_and(|d| d.attached_pic == 1);
                if !cover {
                    video = Some(VideoStream {
                        index: s.index,
                        codec,
                        height: s.height.filter(|h| *h > 0),
                        fps: s
                            .r_frame_rate
                            .as_deref()
                            .and_then(parse_rational)
                            .filter(|f| *f > 0.0),
                        pix_fmt: s.pix_fmt,
                    });
                }
            }
            Some("audio") => audio.push(AudioStream { codec }),
            _ => {}
        }
    }

    Some(MediaInfo {
        duration,
        video,
        audio,
    })
}

/// "60/1" -> 60.0, "30000/1001" -> 29.97, "0/0" -> None.
///
/// `r_frame_rate` comes back as a rational (and `0/0` for streams with no meaningful rate), so it
/// has to be divided rather than parsed as a float.
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

#[cfg(test)]
mod tests {
    use super::*;

    // Real ffprobe output for an MP4 with H.264, AAC and embedded cover art.
    const VIDEO_WITH_COVER: &str = r#"{
        "programs": [],
        "streams": [
            { "index": 0, "codec_name": "h264", "codec_type": "video", "height": 1080,
              "pix_fmt": "yuv420p", "r_frame_rate": "30/1", "disposition": { "attached_pic": 0 } },
            { "index": 1, "codec_name": "aac", "codec_type": "audio", "r_frame_rate": "0/0",
              "disposition": { "attached_pic": 0 } },
            { "index": 2, "codec_name": "mjpeg", "codec_type": "video", "height": 4000,
              "pix_fmt": "yuvj444p", "r_frame_rate": "90000/1", "disposition": { "attached_pic": 1 } }
        ],
        "format": { "duration": "2.090000" }
    }"#;

    #[test]
    fn reads_a_video_file() {
        let info = parse(VIDEO_WITH_COVER).unwrap();
        assert_eq!(info.duration, Some(2.09));
        let v = info.video.unwrap();
        assert_eq!(v.index, 0);
        assert_eq!(v.codec, "h264");
        assert_eq!(v.height, Some(1080));
        assert_eq!(v.fps, Some(30.0));
        assert_eq!(v.pix_fmt.as_deref(), Some("yuv420p"));
        assert_eq!(
            info.audio,
            vec![AudioStream {
                codec: "aac".into()
            }]
        );
    }

    #[test]
    fn cover_art_is_not_video() {
        // An MP3 with album art reports the art as a video stream; treating it as one would
        // compress a song into a one-frame "video" of its sleeve.
        let json = r#"{ "streams": [
            { "index": 0, "codec_name": "mp3", "codec_type": "audio" },
            { "index": 1, "codec_name": "png", "codec_type": "video", "height": 500,
              "r_frame_rate": "90000/1", "disposition": { "attached_pic": 1 } }
        ], "format": { "duration": "120.03" } }"#;
        let info = parse(json).unwrap();
        assert_eq!(info.video, None);
        assert_eq!(info.audio.len(), 1);
    }

    #[test]
    fn missing_fields_are_not_errors() {
        let info = parse(r#"{ "streams": [{ "index": 0, "codec_type": "audio" }] }"#).unwrap();
        assert_eq!(info.duration, None);
        assert_eq!(
            info.audio,
            vec![AudioStream {
                codec: String::new()
            }]
        );
        assert_eq!(
            parse(r#"{ "format": { "duration": "N/A" } }"#)
                .unwrap()
                .duration,
            None
        );
        assert_eq!(parse("not json"), None);
    }

    #[test]
    fn rationals() {
        assert_eq!(parse_rational("60/1"), Some(60.0));
        assert!((parse_rational("30000/1001").unwrap() - 29.97).abs() < 0.01);
        assert_eq!(parse_rational("0/0"), None);
        assert_eq!(parse_rational("garbage"), None);
    }
}
