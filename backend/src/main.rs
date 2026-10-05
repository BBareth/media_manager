mod cleanup;
mod compress;
mod download;
mod encode_plan;
mod jobs;
mod probe;
mod proc;
mod state;
mod transcode;
mod upload;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;

use axum::{
    extract::{DefaultBodyLimit, State},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use state::{AppState, Limits};

/// Default retention: files older than this are deleted by the cleanup sweep.
const DEFAULT_RETENTION_SECS: i64 = 24 * 60 * 60;
/// Upload ceiling for transcode sources (50 GiB).
const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024 * 1024;
/// Encodes that may run at once; see `AppState::acquire_slot` for why it is small.
const DEFAULT_MAX_ENCODES: usize = 2;
/// Downloads are bound by the network, not the CPU, so more of them can share it.
const DEFAULT_MAX_DOWNLOADS: usize = 3;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "media_manager=info,tower_http=warn".into()),
        )
        .init();

    let data_dir = PathBuf::from(env_or("MEDIA_DATA_DIR", "./data"));
    let static_dir = PathBuf::from(env_or("MEDIA_STATIC_DIR", "./static"));
    let retention_secs: i64 = env_parse("MEDIA_RETENTION_SECS", DEFAULT_RETENTION_SECS);
    let port: u16 = env_parse("PORT", 8080);
    let limits = Limits {
        max_encodes: env_parse("MEDIA_MAX_ENCODES", DEFAULT_MAX_ENCODES).max(1),
        max_downloads: env_parse("MEDIA_MAX_DOWNLOADS", DEFAULT_MAX_DOWNLOADS).max(1),
    };

    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!("failed to create data dir {}: {e}", data_dir.display());
        std::process::exit(1);
    }

    let state = AppState::new(data_dir.clone(), retention_secs, limits);

    // Background TTL cleanup.
    tokio::spawn(cleanup::run_cleanup(state.clone()));

    let api = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/info", get(info))
        .route("/events", get(jobs::events))
        .route("/downloads", post(download::start_download))
        .route("/transcode", post(transcode::start_transcode))
        .route("/compress", post(compress::start_compress))
        .route("/jobs", get(jobs::list_jobs))
        .route("/jobs/{id}", get(jobs::get_job).delete(jobs::delete_job))
        .route("/jobs/{id}/file", get(jobs::download_output))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES));

    // Serve the built frontend, falling back to index.html for SPA routing.
    let index = static_dir.join("index.html");
    let static_service = ServeDir::new(&static_dir)
        .precompressed_gzip()
        .not_found_service(ServeFile::new(index));

    let app = Router::new()
        .nest("/api", api)
        .fallback_service(static_service)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("cannot bind {addr}: {e}"));

    tracing::info!("media_manager listening on http://{addr}");
    tracing::info!("data dir: {}", data_dir.display());
    tracing::info!("retention: {retention_secs}s");
    tracing::info!(
        "at most {} encodes and {} downloads at once",
        limits.max_encodes,
        limits.max_downloads
    );

    axum::serve(listener, app).await.expect("server error");
}

/// What the UI needs to describe this server accurately.
#[derive(Serialize)]
struct Info {
    retention_secs: i64,
    max_upload_bytes: usize,
    #[serde(flatten)]
    limits: Limits,
}

/// GET /api/info
async fn info(State(state): State<AppState>) -> Json<Info> {
    Json(Info {
        retention_secs: state.retention_secs,
        max_upload_bytes: MAX_UPLOAD_BYTES,
        limits: state.limits,
    })
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

/// Read and parse an environment variable, falling back to `default` when it is unset or
/// unparseable.
fn env_parse<T: FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}
