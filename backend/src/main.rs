mod cleanup;
mod compress;
mod download;
mod jobs;
mod proc;
mod state;
mod transcode;

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::{
    extract::DefaultBodyLimit,
    routing::{delete, get, post},
    Router,
};
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use state::AppState;

/// Default retention: files older than this are deleted by the cleanup sweep.
const DEFAULT_RETENTION_SECS: i64 = 24 * 60 * 60;
/// Upload ceiling for transcode sources (50 GiB).
const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024 * 1024;

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
    let retention_secs = std::env::var("MEDIA_RETENTION_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_RETENTION_SECS);
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8080);

    if let Err(e) = std::fs::create_dir_all(&data_dir) {
        eprintln!("failed to create data dir {}: {e}", data_dir.display());
        std::process::exit(1);
    }

    let state = AppState::new(data_dir.clone(), retention_secs);

    // Background TTL cleanup.
    tokio::spawn(cleanup::run_cleanup(state.clone()));

    let api = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/downloads", post(download::start_download))
        .route("/transcode", post(transcode::start_transcode))
        .route("/compress", post(compress::start_compress))
        .route("/jobs", get(jobs::list_jobs))
        .route("/jobs/{id}", get(jobs::get_job))
        .route("/jobs/{id}", delete(jobs::delete_job))
        .route("/jobs/{id}/file", get(jobs::download_output))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES));

    // Serve the built frontend, falling back to index.html for SPA routing.
    let index = static_dir.join("index.html");
    let static_service = ServeDir::new(&static_dir).not_found_service(ServeFile::new(index));

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

    axum::serve(listener, app)
        .await
        .expect("server error");
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
