# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Live progress. The page receives job updates as server-sent events from
  `/api/events` instead of polling every 1.5 seconds, and running jobs show an
  estimate of the time left.
- A queue. At most `MEDIA_MAX_ENCODES` (default 2) conversions and
  compressions and `MEDIA_MAX_DOWNLOADS` (default 3) downloads run at once; the
  rest wait as *Queued*. Two 1080p encodes side by side on four cores both
  finish at ~20.5 s; one after the other, the first is ready at ~11 s.
- Compress handles audio files: AAC at the bitrate the target allows, mono when
  the budget is tight. The form always accepted audio, but the job failed with
  ffmpeg's "Output file does not contain any stream".
- Preview finished video, audio and images in the browser, and resume
  interrupted downloads: the file endpoint answers `Range` requests.
- Upload progress with speed and time left, and a way to cancel uploads.
- Convert takes several files at once, like Compress.
- One-click size presets on Compress, and a warning before uploading a file that
  is already under the target.
- YouTube support in the image: yt-dlp now ships with the Deno JavaScript
  runtime it needs to answer YouTube's challenges.
- A light theme that follows the system setting, keyboard navigation between
  tabs, screen-reader announcements when a job finishes, and a layout that
  works on a phone.

### Changed

- Converting no longer re-encodes streams the new container can hold as they
  are. A 20 s 1080p H.264 MKV became an MP4 in 0.18 s instead of 15.4 s, and
  the picture is the original rather than a re-encode of it.
- WebM conversions run libvpx-vp9 at `-cpu-used 4` instead of its default of 1:
  three times faster (16.5 s against 49.6 s for a 6 s 1080p clip) for a file
  6 % larger at the same SSIM.
- Image compression shrinks the picture before dropping below a sensible
  quality, and binary-searches instead of trying every setting in turn. For
  JPEG over eight targets it needed 54 encodes instead of 121, and the result
  scored a higher SSIM at every target but the smallest, where both agree; WebP
  took 40 instead of 60 at the same quality.
- Downloads fetch four fragments at a time from DASH and HLS sources.
- Each source is probed with one `ffprobe` call instead of up to three.
- Button colours meet WCAG AA contrast; white on the old blue was 3.2:1.

### Fixed

- Deleting a running job left its `ffmpeg` or `yt-dlp` process running to the
  end, writing into a directory that no longer existed. It is now stopped.
- A quality limit on a link straight to a video file failed with "Requested
  format is not available", because the file reports no height. The limit is
  now a preference: the largest at or under it, or the closest above.
- Asking for MKV from a site that serves a single combined file returned the
  site's container instead; the file is now remuxed into the one requested.
- A failed or abandoned upload left its partial file on disk until the next
  retention sweep.
- An uploaded filename long enough to exceed the filesystem's limit once an
  extension was added failed the job at the very end; output files now have
  fixed names on disk.

## [0.1.1] - 2026-09-17

### Changed

- The frontend moves to React 19, Vite 8 and TypeScript 7. No behaviour
  changes; the production bundle grows from 156 kB to 233 kB (50 kB to 73 kB
  gzipped), which is React 19's runtime.

### Fixed

- Video compression could finish above its target — an 8 s 720p source asked
  for 0.5 MB came back at 0.52 MB, because x264 does not promise to hit the
  bitrate it is given and the flat 3 % margin did not cover it. The encode is
  now measured and repeated at a corrected bitrate when it lands over, up to
  three attempts, reusing the first pass so a retry costs one encode rather
  than two. A target that cannot be reached fails with how close it got
  instead of quietly handing back an oversized file.

## [0.1.0] - 2026-09-17

First public release.

### Added

- **Download** tab — fetch video or audio from any site `yt-dlp` supports, with
  a choice of container (MP4, MKV, WebM, MP3, M4A, OPUS, OGG, WAV, FLAC) and a
  quality ceiling up to 4K.
- **Transcode** tab — convert an uploaded file to another format with `ffmpeg`,
  with a real progress bar derived from the source duration via `ffprobe`.
- **Compress** tab — hit a target file size. The encoder picks a resolution and
  frame rate that the target bitrate can actually support instead of re-encoding
  at the source's own dimensions, which is both faster and less blocky.
- Automatic cleanup: a sweep every 30 minutes deletes job folders older than
  `MEDIA_RETENTION_SECS`, so the data volume cannot grow without bound.
- Multi-architecture container images (`linux/amd64`, `linux/arm64`) published
  to GitHub Container Registry.

### Security

- Download URLs are restricted to `http://` and `https://`; `file://` and other
  schemes that `yt-dlp` would otherwise resolve are rejected.
- The URL is passed after a `--` terminator so a value starting with a dash
  cannot be reinterpreted as a `yt-dlp` option such as `--exec`.
- The container runs as uid 10001 rather than root. A bind-mounted data
  directory must be writable by that uid —
  `sudo chown -R 10001:10001 <dir>`. Named volumes need nothing.

[Unreleased]: https://github.com/BBareth/media_manager/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/BBareth/media_manager/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/BBareth/media_manager/releases/tag/v0.1.0
