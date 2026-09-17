# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/BBareth/media_manager/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/BBareth/media_manager/releases/tag/v0.1.0
