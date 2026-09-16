# syntax=docker/dockerfile:1

# ---- Stage 1: build the TypeScript/React frontend ----
FROM node:26-slim AS frontend
WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json* ./
RUN npm install
COPY frontend/ ./
RUN npm run build

# ---- Stage 2: build the Rust backend ----
FROM rust:1-slim-bookworm AS backend
WORKDIR /app
# Cache dependencies first using a throwaway main.rs.
COPY backend/Cargo.toml backend/Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src
COPY backend/src ./src
# Touch so cargo rebuilds with the real sources.
RUN touch src/main.rs && cargo build --release

# ---- Stage 3: runtime ----
FROM debian:bookworm-slim AS runtime

# yt-dlp is installed from PyPI into an isolated venv and runs on real Python.
# This avoids the fragile self-extraction the standalone PyInstaller binary does
# at runtime (which fails on constrained /tmp with "decompression returned -1").
ARG YTDLP_VERSION=latest
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ffmpeg \
        ca-certificates \
        python3 \
        python3-venv \
    && python3 -m venv /opt/ytdlp \
    && /opt/ytdlp/bin/pip install --no-cache-dir --upgrade pip \
    && if [ "$YTDLP_VERSION" = "latest" ]; then \
         /opt/ytdlp/bin/pip install --no-cache-dir --upgrade "yt-dlp[default]"; \
       else \
         /opt/ytdlp/bin/pip install --no-cache-dir "yt-dlp[default]==${YTDLP_VERSION}"; \
       fi \
    && ln -s /opt/ytdlp/bin/yt-dlp /usr/local/bin/yt-dlp \
    && rm -rf /var/lib/apt/lists/*

LABEL org.opencontainers.image.title="media_manager" \
      org.opencontainers.image.description="Self-hosted download, transcode and size-targeted compression for media." \
      org.opencontainers.image.source="https://github.com/BBareth/media_manager" \
      org.opencontainers.image.licenses="MIT"

# Nothing here needs root, and this container spends its time feeding untrusted
# media to ffmpeg and yt-dlp. A fixed uid keeps bind-mounted data directories
# predictable to chown from the host.
RUN groupadd --system --gid 10001 app \
    && useradd --system --uid 10001 --gid app --home-dir /app --shell /usr/sbin/nologin app

WORKDIR /app
COPY --from=backend /app/target/release/media_manager /app/media_manager
COPY --from=frontend /app/frontend/dist /app/static

ENV MEDIA_DATA_DIR=/data \
    MEDIA_STATIC_DIR=/app/static \
    MEDIA_RETENTION_SECS=86400 \
    PORT=8080

RUN mkdir -p /data && chown -R app:app /data /app
VOLUME ["/data"]
EXPOSE 8080
USER app

# Honours $PORT rather than hard-coding 8080, so a remapped port stays healthy.
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD ["python3", "-c", "import os,sys,urllib.request; p=os.environ.get('PORT','8080'); sys.exit(0 if urllib.request.urlopen(f'http://127.0.0.1:{p}/api/health', timeout=3).read()==b'ok' else 1)"]

CMD ["/app/media_manager"]
