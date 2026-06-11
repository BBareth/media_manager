use std::fs;

use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use tokio_util::io::ReaderStream;

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

/// DELETE /api/jobs/{id} — remove the job and delete all of its files.
pub async fn delete_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> StatusCode {
    match state.remove_job(&id) {
        Some(job) => {
            let _ = fs::remove_dir_all(&job.dir);
            StatusCode::NO_CONTENT
        }
        None => StatusCode::NOT_FOUND,
    }
}

/// GET /api/jobs/{id}/file — stream the finished output as an attachment.
pub async fn download_output(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let Some(job) = state.get_job(&id) else {
        return (StatusCode::NOT_FOUND, "job not found").into_response();
    };
    if job.status != JobStatus::Completed {
        return (StatusCode::CONFLICT, "job is not finished").into_response();
    }
    let Some(path) = job.output_path.clone() else {
        return (StatusCode::NOT_FOUND, "no output file").into_response();
    };

    let file = match tokio::fs::File::open(&path).await {
        Ok(f) => f,
        Err(_) => return (StatusCode::NOT_FOUND, "output file missing").into_response(),
    };

    let name = job.output_name.clone().unwrap_or_else(|| "download".to_string());
    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    if let Some(size) = job.output_size {
        if let Ok(v) = HeaderValue::from_str(&size.to_string()) {
            headers.insert(header::CONTENT_LENGTH, v);
        }
    }
    if let Ok(v) = HeaderValue::from_str(&content_disposition(&name)) {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }

    (headers, body).into_response()
}

/// Build a safe `Content-Disposition` header that preserves Unicode names via
/// the RFC 5987 `filename*` form while keeping an ASCII fallback.
fn content_disposition(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| if c.is_ascii() && c != '"' && c != '\\' && !c.is_control() { c } else { '_' })
        .collect();
    format!(
        "attachment; filename=\"{}\"; filename*=UTF-8''{}",
        ascii,
        percent_encode(name)
    )
}

/// Percent-encode everything outside the RFC 3986 unreserved set.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        let unreserved = byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if unreserved {
            out.push(byte as char);
        } else {
            out.push('%');
            out.push_str(&format!("{byte:02X}"));
        }
    }
    out
}
