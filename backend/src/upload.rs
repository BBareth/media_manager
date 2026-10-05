//! The parts of the transcode and compress endpoints that are about receiving a file rather than
//! about what happens to it afterwards.

use std::path::{Path, PathBuf};

use axum::extract::multipart::{Field, MultipartError};
use axum::http::StatusCode;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::state::AppState;

pub type ApiError = (StatusCode, String);

pub fn bad(msg: impl Into<String>) -> ApiError {
    (StatusCode::BAD_REQUEST, msg.into())
}

pub fn internal(msg: impl Into<String>) -> ApiError {
    (StatusCode::INTERNAL_SERVER_ERROR, msg.into())
}

/// Keep the status axum picked: an over-limit body is a 413, a dropped connection a 400.
pub fn multipart_error(e: MultipartError) -> ApiError {
    (e.status(), format!("invalid upload: {}", e.body_text()))
}

/// A fresh job directory that removes itself unless the request gets as far as starting a job.
///
/// Without this every early return — a bad field, a dropped connection half-way through a 10 GB
/// upload — left its partial file on disk until the retention sweep, a day later by default.
pub struct JobDir {
    id: String,
    path: PathBuf,
    keep: bool,
}

impl JobDir {
    pub fn create(state: &AppState) -> Result<Self, ApiError> {
        let id = Uuid::new_v4().to_string();
        let path = state.job_dir(&id);
        std::fs::create_dir_all(&path).map_err(|e| internal(format!("mkdir failed: {e}")))?;
        Ok(Self {
            id,
            path,
            keep: false,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Hand the directory over to a job; it is no longer cleaned up on drop.
    pub fn keep(mut self) -> (String, PathBuf) {
        self.keep = true;
        (std::mem::take(&mut self.id), std::mem::take(&mut self.path))
    }
}

impl Drop for JobDir {
    fn drop(&mut self) {
        if self.keep {
            return;
        }
        let path = std::mem::take(&mut self.path);
        // A partial upload can be large; unlinking it is not work for an async worker thread.
        match tokio::runtime::Handle::try_current() {
            Ok(rt) => {
                rt.spawn_blocking(move || std::fs::remove_dir_all(path));
            }
            Err(_) => {
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }
}

/// A file received from a multipart upload.
pub struct Upload {
    pub path: PathBuf,
    /// The name the client gave it. Display only — never part of a path.
    pub original_name: String,
    /// Lower-case, alphanumeric extension of `original_name`, or "" if it had none.
    pub ext: String,
    pub size: u64,
}

/// Stream a multipart file field to disk inside `dir`.
pub async fn save_file(mut field: Field<'_>, dir: &Path) -> Result<Upload, ApiError> {
    let original_name = field
        .file_name()
        .map(str::to_string)
        .unwrap_or_else(|| "upload".to_string());
    let ext = safe_extension(&original_name);
    let path = dir.join(if ext.is_empty() {
        "source.bin".to_string()
    } else {
        format!("source.{ext}")
    });

    let mut file = tokio::fs::File::create(&path)
        .await
        .map_err(|e| internal(format!("cannot create upload file: {e}")))?;
    let mut size: u64 = 0;
    while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
        size += chunk.len() as u64;
        file.write_all(&chunk)
            .await
            .map_err(|e| internal(format!("write error: {e}")))?;
    }
    file.flush()
        .await
        .map_err(|e| internal(format!("flush error: {e}")))?;

    Ok(Upload {
        path,
        original_name,
        ext,
        size,
    })
}

/// Read a small text field.
pub async fn text(field: Field<'_>) -> Result<String, ApiError> {
    field
        .text()
        .await
        .map(|t| t.trim().to_string())
        .map_err(multipart_error)
}

/// Interpret a form checkbox value.
pub fn truthy(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        "true" | "1" | "on" | "yes"
    )
}

/// The extension of a client-supplied filename, reduced to something that is safe to put in a
/// path we build. It only ever reaches `source.<ext>` inside the job's own directory, but a name
/// that came from a request does not get to choose any characters of a path unchecked.
pub fn safe_extension(name: &str) -> String {
    let ext = Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if !ext.is_empty() && ext.len() <= 10 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        ext
    } else {
        String::new()
    }
}

/// The filename without its extension, for naming the output.
pub fn stem(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "output".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_are_normalised() {
        assert_eq!(safe_extension("Clip.MKV"), "mkv");
        assert_eq!(safe_extension("archive.tar.gz"), "gz");
        assert_eq!(safe_extension("no_extension"), "");
    }

    #[test]
    fn hostile_extensions_are_dropped() {
        assert_eq!(safe_extension("x.mp4 ; rm -rf"), "");
        assert_eq!(safe_extension("x.m\u{0}p4"), "");
        assert_eq!(safe_extension("x.averyveryverylongext"), "");
        // The last component is all that counts, so separators cannot sneak in.
        assert_eq!(safe_extension("a.mp4/../../etc/passwd"), "");
    }

    #[test]
    fn checkbox_values() {
        for yes in ["true", "TRUE", "1", "on", "yes"] {
            assert!(truthy(yes), "{yes}");
        }
        for no in ["false", "0", "", "off", "maybe"] {
            assert!(!truthy(no), "{no}");
        }
    }

    #[test]
    fn stems_fall_back_to_output() {
        assert_eq!(stem("holiday.mov"), "holiday");
        assert_eq!(stem(".mp4"), ".mp4");
        assert_eq!(stem(""), "output");
    }
}
