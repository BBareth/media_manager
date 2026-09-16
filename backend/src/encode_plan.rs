//! Deciding *what* to encode before deciding how hard to work at it.
//!
//! The compressor used to hand ffmpeg the source at its own resolution and frame rate whatever
//! the size target was, and that is both the slowest and the worst-looking choice available.
//!
//! Measured on a real 2560x1440 60 fps game capture, 60 seconds targeted at 12 MB, on the six
//! cores this container gets:
//!
//! ```text
//!   1440p60 preset medium   95 s   0.0068 bits per pixel   <- what it did
//!   1080p60 preset medium   71 s   0.0121 bpp
//!   1080p30 preset medium   50 s   0.0241 bpp
//!   1080p30 preset faster   43 s   0.0241 bpp
//!    720p30 preset faster   33 s   0.0543 bpp
//! ```
//!
//! Two things fall out of that. The encode time tracks pixels per second almost linearly, so the
//! resolution and frame rate — not the x264 preset — are what make it slow. And at 0.0068 bits
//! per pixel a 1440p frame is being asked to describe four million pixels with almost nothing;
//! the result is blocky *because* it kept the resolution. Scaling down is not a compromise here,
//! it is what makes the picture better and the wait shorter at the same time.
//!
//! So: pick the largest rung of a normal resolution ladder whose bits-per-pixel lands in a range
//! x264 can actually work with, never upscale, and only halve the frame rate when the budget is
//! too thin to carry it.

/// Bits per pixel below which x264 starts visibly falling apart on real footage. Chosen from the
/// measurements above: 1440p60 at 0.0068 was unusable, 1080p30 at 0.024 was fine.
const MIN_BPP: f64 = 0.02;

/// Standard heights, largest first. The source's own height is never exceeded.
const LADDER: &[u32] = &[2160, 1440, 1080, 900, 720, 540, 480, 360];

/// The smallest height worth producing — below this the video is not worth watching and the
/// bitrate is better spent on the frames that remain.
const FLOOR_HEIGHT: u32 = 360;

#[derive(Debug, Clone, PartialEq)]
pub struct EncodePlan {
    /// Target height, or `None` to keep the source's.
    pub height: Option<u32>,
    /// Target frame rate, or `None` to keep the source's.
    pub fps: Option<u32>,
    /// x264 preset.
    pub preset: &'static str,
}

impl EncodePlan {
    /// The `-vf` value, or `None` when nothing needs filtering.
    ///
    /// `scale=-2:H` keeps the aspect ratio and rounds the width to an even number, which h264
    /// requires with yuv420p — `-1` can land on an odd width and fails the encode outright.
    pub fn video_filter(&self) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        if let Some(h) = self.height {
            parts.push(format!("scale=-2:{h}"));
        }
        if let Some(f) = self.fps {
            parts.push(format!("fps={f}"));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(","))
        }
    }
}

/// Work out how to spend the bitrate budget.
///
/// `src_height` / `src_fps` come from ffprobe; pass `None` when they could not be read, in which
/// case the source is left alone rather than guessed at.
pub fn plan_encode(video_bps: f64, src_height: Option<u32>, src_fps: Option<f64>) -> EncodePlan {
    // The preset is worth a fifth of the time and costs very little quality at these bitrates.
    // "faster" rather than "veryfast": veryfast starts dropping enough tools that the picture
    // suffers at exactly the low bitrates this feature exists for.
    let preset = "faster";

    let (Some(src_h), Some(src_fps)) = (src_height, src_fps) else {
        return EncodePlan {
            height: None,
            fps: None,
            preset,
        };
    };
    if src_h == 0 || src_fps <= 0.0 {
        return EncodePlan {
            height: None,
            fps: None,
            preset,
        };
    }

    // 16:9 is the assumption behind the ladder, and it only feeds the bpp estimate — a 4:3 source
    // is judged slightly conservatively, which errs toward the sharper result.
    let bpp_at = |h: u32, fps: f64| video_bps / (h as f64 * (h as f64 * 16.0 / 9.0) * fps);

    // Frame rate is decided first and once. Halving 60 to 30 frees as much budget as dropping a
    // whole resolution rung and is far less visible on anything but fast motion, so it is the
    // cheaper concession — but only when the source cannot carry its own resolution at full rate.
    // Searching the ladder at 60 first would find 720p60 before ever considering 1080p30, which
    // is the worse of the two at the same bitrate.
    let halved_fps = if src_fps > 45.0 && bpp_at(src_h, src_fps) < MIN_BPP {
        Some((src_fps / 2.0).round() as u32)
    } else {
        None
    };
    let fps = halved_fps.map(|f| f as f64).unwrap_or(src_fps);

    for &h in LADDER {
        if h > src_h {
            continue; // never upscale
        }
        if bpp_at(h, fps) >= MIN_BPP {
            return EncodePlan {
                height: if h == src_h { None } else { Some(h) },
                fps: halved_fps,
                preset,
            };
        }
    }

    // Nothing on the ladder fits the budget: take the floor and the lower frame rate. The result
    // will be small and soft, which is what a very small target actually means.
    EncodePlan {
        height: if src_h <= FLOOR_HEIGHT {
            None
        } else {
            Some(FLOOR_HEIGHT)
        },
        fps: halved_fps,
        preset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A generous budget should change nothing — the point is to avoid pointless work, not to
    // shrink everything on principle.
    #[test]
    fn a_roomy_budget_leaves_the_source_alone() {
        let p = plan_encode(20_000_000.0, Some(1440), Some(60.0));
        assert_eq!(p.height, None);
        assert_eq!(p.fps, None);
        assert_eq!(p.video_filter(), None);
    }

    // The measured case: 1440p60 at ~1.4 Mbps was 0.0068 bpp and 95 seconds.
    #[test]
    fn a_thin_budget_scales_the_real_world_case_down() {
        let p = plan_encode(1_400_000.0, Some(1440), Some(60.0));
        assert!(p.height.is_some(), "1440p at 1.4 Mbps has to come down");
        assert!(p.height.unwrap() <= 1080);
        assert_eq!(
            p.fps,
            Some(30),
            "60 fps is the cheapest thing to give up first"
        );
    }

    #[test]
    fn frame_rate_goes_before_resolution() {
        // At this budget 1080p is reachable, but only once 60 fps has been halved.
        let p = plan_encode(1_600_000.0, Some(1440), Some(60.0));
        assert_eq!(p.fps, Some(30));
        assert_eq!(p.height, Some(1080));
    }

    #[test]
    fn a_thirty_fps_source_keeps_its_frame_rate() {
        // Halving 30 fps would look like a slideshow; only high frame rates are candidates.
        let p = plan_encode(300_000.0, Some(1080), Some(30.0));
        assert_eq!(p.fps, None);
        assert!(p.height.unwrap() < 1080);
    }

    #[test]
    fn it_never_upscales() {
        // A tiny source with a huge budget must not be blown up to 2160.
        let p = plan_encode(50_000_000.0, Some(480), Some(30.0));
        assert_eq!(p.height, None);
    }

    #[test]
    fn a_hopeless_budget_stops_at_the_floor() {
        let p = plan_encode(20_000.0, Some(1440), Some(60.0));
        assert_eq!(p.height, Some(FLOOR_HEIGHT));
        assert_eq!(p.fps, Some(30));
    }

    #[test]
    fn an_unreadable_source_is_left_untouched() {
        // ffprobe failing is a reason to do nothing clever, not a reason to guess.
        assert_eq!(plan_encode(500_000.0, None, Some(60.0)).height, None);
        assert_eq!(plan_encode(500_000.0, Some(1080), None).height, None);
        assert_eq!(plan_encode(500_000.0, Some(0), Some(60.0)).height, None);
        assert_eq!(plan_encode(500_000.0, Some(1080), Some(0.0)).height, None);
    }

    #[test]
    fn the_filter_keeps_the_aspect_ratio_and_an_even_width() {
        // -2 rounds the width to a multiple of two; -1 can produce an odd width, which h264
        // with yuv420p rejects and the whole encode fails.
        let p = EncodePlan {
            height: Some(1080),
            fps: Some(30),
            preset: "faster",
        };
        assert_eq!(p.video_filter().as_deref(), Some("scale=-2:1080,fps=30"));

        let only_scale = EncodePlan {
            height: Some(720),
            fps: None,
            preset: "faster",
        };
        assert_eq!(only_scale.video_filter().as_deref(), Some("scale=-2:720"));

        let only_fps = EncodePlan {
            height: None,
            fps: Some(30),
            preset: "faster",
        };
        assert_eq!(only_fps.video_filter().as_deref(), Some("fps=30"));
    }

    #[test]
    fn every_rung_it_can_pick_is_a_real_resolution() {
        for bps in [50_000.0, 200_000.0, 800_000.0, 3_000_000.0, 12_000_000.0] {
            let p = plan_encode(bps, Some(2160), Some(60.0));
            if let Some(h) = p.height {
                assert!(
                    LADDER.contains(&h),
                    "{h} is not on the ladder (at {bps} bps)"
                );
                assert!(h % 2 == 0, "{h} would give an odd dimension");
            }
        }
    }
}
