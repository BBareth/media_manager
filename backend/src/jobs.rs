use std::convert::Infallible;
use std::time::Duration;

use axum::{
    body::Body,
    extract::{Path, Query, Request, State},
    http::{header, HeaderValue, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    Json,
};
use futures_util::stream;
use serde::Deserialize;
use tower_http::services::ServeFile;

use crate::state::{AppState, Job, JobStatus};

/// GET /api/jobs — every job, newest first.
pub async fn list_jobs(State(state): State<AppState>) -> Json<Vec<Job>> {
    Json(state.list_jobs())
}

/// GET /api/jobs/{id} — a single job.
pub async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Job>, StatusCode> {
    state.get_job(&id).map(Json).ok_or(StatusCode::NOT_FOUND)
}

/// DELETE /api/jobs/{id} — stop the job if it is running, and delete all of its files.
pub async fn delete_job(State(state): State<AppState>, Path(id): Path<String>) -> StatusCode {
    match state.remove_job(&id) {
        Some(job) => {
            let _ = tokio::fs::remove_dir_all(&job.dir).await;
            StatusCode::NO_CONTENT
        }
        None => StatusCode::NOT_FOUND,
    }
}

/// How long to let a burst of changes settle before sending the job list again. An encode
/// reports progress twice a second and yt-dlp many times more; nobody needs the bar redrawn
/// faster than this, and every event is the whole list.
const EVENT_THROTTLE: Duration = Duration::from_millis(250);

/// GET /api/events — the job list as server-sent events: once on connect, then again whenever
/// anything changes.
///
/// The UI used to poll `/api/jobs` every 1.5 s for as long as the page was open, busy or idle,
/// visible or not, and still showed progress up to 1.5 s late. Pushing costs nothing while
/// nothing happens and shows changes within a quarter of a second.
pub async fn events(State(state): State<AppState>) -> impl IntoResponse {
    let rx = state.subscribe();
    let updates = stream::unfold((state, rx, true), |(state, mut rx, first)| async move {
        if !first {
            rx.changed().await.ok()?;
            tokio::time::sleep(EVENT_THROTTLE).await;
        }
        // Mark everything up to now as seen *before* reading, so a change that lands while the
        // list is serialised triggers another event rather than being lost.
        rx.borrow_and_update();
        let event = Event::default()
            .event("jobs")
            .json_data(state.list_jobs())
            .ok()?;
        Some((Ok::<_, Infallible>(event), (state, rx, false)))
    });
    (
        // Tell nginx and friends not to buffer the stream into uselessness.
        [("x-accel-buffering", "no")],
        Sse::new(updates).keep_alive(KeepAlive::default()),
    )
}

#[derive(Deserialize)]
pub struct FileQuery {
    /// Ask for the file to be shown in the browser instead of saved.
    #[serde(default)]
    inline: bool,
}

/// GET /api/jobs/{id}/file — the finished output.
///
/// Served by tower-http, which answers Range requests: a browser can resume an interrupted
/// download, and the preview player can seek. It also reads in 64 KiB chunks where the
/// hand-rolled stream used tokio's 4 KiB default.
pub async fn download_output(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<FileQuery>,
    req: Request,
) -> Response {
    let Some(job) = state.get_job(&id) else {
        return (StatusCode::NOT_FOUND, "job not found").into_response();
    };
    if job.status != JobStatus::Completed {
        return (StatusCode::CONFLICT, "job is not finished").into_response();
    }
    let Some(path) = job.output_path else {
        return (StatusCode::NOT_FOUND, "no output file").into_response();
    };

    let mut res = match ServeFile::new(&path).try_call(req).await {
        Ok(res) if res.status() != StatusCode::NOT_FOUND => res.map(Body::new),
        _ => return (StatusCode::NOT_FOUND, "output file missing").into_response(),
    };

    let content_type = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let inline = query.inline && previewable(content_type);
    let name = job.output_name.unwrap_or_else(|| "download".to_string());
    let headers = res.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&content_disposition(&name, inline)) {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }
    // The type is guessed from an extension that, for downloads, the remote site chose. Never
    // let the browser second-guess it into something executable.
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    res
}

/// Whether a file may be shown inline on this origin. Media only: an HTML or SVG output shown
/// inline would run script with full access to the API.
fn previewable(content_type: &str) -> bool {
    let ct = content_type.to_ascii_lowercase();
    ct.starts_with("video/")
        || ct.starts_with("audio/")
        || (ct.starts_with("image/") && !ct.starts_with("image/svg"))
}

/// Build a safe `Content-Disposition` header that preserves Unicode names via
/// the RFC 5987 `filename*` form while keeping an ASCII fallback.
fn content_disposition(name: &str, inline: bool) -> String {
    let ascii: String = name
        .chars()
        .map(|c| {
            if c.is_ascii() && c != '"' && c != '\\' && !c.is_control() {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "{}; filename=\"{}\"; filename*=UTF-8''{}",
        if inline { "inline" } else { "attachment" },
        ascii,
        percent_encode(name)
    )
}

/// Percent-encode everything outside the RFC 3986 unreserved set.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        let unreserved = byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if unreserved {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_media_is_shown_inline() {
        assert!(previewable("video/mp4"));
        assert!(previewable("audio/mpeg"));
        assert!(previewable("image/jpeg"));
        assert!(!previewable("image/svg+xml"));
        assert!(!previewable("text/html"));
        assert!(!previewable("application/octet-stream"));
        assert!(!previewable(""));
    }

    #[test]
    fn content_disposition_escapes_names() {
        assert_eq!(
            content_disposition("clip \"1\".mp4", false),
            "attachment; filename=\"clip _1_.mp4\"; filename*=UTF-8''clip%20%221%22.mp4"
        );
        assert_eq!(
            content_disposition("Grüße.mp3", true),
            "inline; filename=\"Gr__e.mp3\"; filename*=UTF-8''Gr%C3%BC%C3%9Fe.mp3"
        );
    }
}
