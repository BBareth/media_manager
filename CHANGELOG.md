# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
