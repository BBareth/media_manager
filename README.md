# Media Manager

A self-hosted media manager with two tools:

1. **Download** — grab videos/audio from YouTube and hundreds of other sites
   (powered by `yt-dlp`). Pick the container/format (MP4, MKV, WebM, MP3, M4A,
   OPUS, OGG, WAV, FLAC) and the quality (up to 4K). When a download finishes you
   pull it straight to your browser, then delete it from the server.
2. **Transcode** — convert any audio/video file to another format with `ffmpeg`
   (e.g. AVI → MP4, MP3 → OGG, MKV → MP4, anything ffmpeg supports here).

Everything is transient: files older than the retention window (1 day by
default) are deleted automatically, so the server never fills up.

- **Backend:** Rust + Axum (async, single small static binary).
- **Frontend:** TypeScript + React + Vite.
- **Tools:** `yt-dlp` and `ffmpeg`, bundled in the Docker image.

> Built for trusted local/LAN use — there is **no authentication**. Don't expose
> it directly to the internet.

---

## Run it with Docker (recommended)

### Portainer

1. In Portainer go to **Stacks → Add stack**.
2. Paste the contents of [`docker-compose.yml`](./docker-compose.yml).
3. Deploy. Open `http://<host>:8080`.

### docker compose / CLI

```bash
docker compose up -d
# then open http://localhost:8080
```

### Plain docker run

```bash
docker run -d --name media_manager \
  -p 8080:8080 \
  -v media_data:/data \
  -e MEDIA_RETENTION_SECS=86400 \
  --restart unless-stopped \
  bareth31/epsilon:media_manager
```

## Configuration

| Env var                | Default        | Meaning                                            |
| ---------------------- | -------------- | -------------------------------------------------- |
| `PORT`                 | `8080`         | HTTP port inside the container.                    |
| `MEDIA_RETENTION_SECS` | `86400` (1 day)| Age at which files & jobs are auto-deleted.        |
| `MEDIA_DATA_DIR`       | `/data`        | Where working files are stored (mount a volume).   |
| `MEDIA_STATIC_DIR`     | `/app/static`  | Built frontend assets (set inside the image).      |

The `/data` volume is scratch space — it does not need backups; its contents are
purged automatically.

---

## Building & pushing the image

```bash
# from the repo root
docker build -t bareth31/epsilon:media_manager .

docker login            # log in to Docker Hub as bareth31
docker push bareth31/epsilon:media_manager
```

A pinned yt-dlp version can be baked in:

```bash
docker build --build-arg YTDLP_VERSION=2025.05.22 -t bareth31/epsilon:media_manager .
```

---

## Local development (without Docker)

You need: Rust, Node 20+, plus `ffmpeg`, `ffprobe`, and `yt-dlp` on your `PATH`.

```bash
# terminal 1 — backend on :8080
cd backend
cargo run

# terminal 2 — frontend dev server on :5173 (proxies /api to :8080)
cd frontend
npm install
npm run dev
```

For a production-style local run, build the frontend and point the backend at it:

```bash
cd frontend && npm run build && cd ..
cd backend
# PowerShell:
$env:MEDIA_STATIC_DIR="../frontend/dist"; cargo run --release
# bash:
MEDIA_STATIC_DIR=../frontend/dist cargo run --release
```

---

## How it works

- Each job gets its own folder under `/data/<job-id>/`.
- Downloads run `yt-dlp` with a format selector derived from your quality choice
  and `--merge-output-format` / `--audio-format` for the container.
- Transcodes stream your upload to disk, probe its duration with `ffprobe` for an
  accurate progress bar, then run `ffmpeg` with sensible codecs per target format.
- The frontend polls `/api/jobs` for live progress.
- A background sweep every 30 minutes deletes any job folder older than
  `MEDIA_RETENTION_SECS` (and removes stray folders from earlier runs).

## API

| Method   | Path                  | Purpose                              |
| -------- | --------------------- | ------------------------------------ |
| `POST`   | `/api/downloads`      | `{ url, format, quality }` → job     |
| `POST`   | `/api/transcode`      | multipart `file` + `format` → job    |
| `GET`    | `/api/jobs`           | list all jobs (newest first)         |
| `GET`    | `/api/jobs/{id}`      | one job                              |
| `GET`    | `/api/jobs/{id}/file` | download the finished output         |
| `DELETE` | `/api/jobs/{id}`      | delete a job and its files           |
