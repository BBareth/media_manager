use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;

/// What kind of work a job represents.
#[derive(Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobKind {
    Download,
    Transcode,
    Compress,
}

/// Lifecycle of a job.
#[derive(Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

/// A single unit of work plus everything the UI needs to render it.
#[derive(Clone, Serialize)]
pub struct Job {
    pub id: String,
    pub kind: JobKind,
    pub status: JobStatus,
    /// 0.0 – 100.0
    pub progress: f32,
    /// Primary label (source URL or uploaded filename).
    pub title: String,
    /// Secondary label, e.g. "1080p · mp4" or "avi → mp4".
    pub detail: String,
    /// Suggested filename when downloading the finished output.
    pub output_name: Option<String>,
    /// Size of the finished output in bytes.
    pub output_size: Option<u64>,
    pub error: Option<String>,
    /// Unix seconds.
    pub created_at: i64,

    // --- fields not exposed to the API ---
    #[serde(skip)]
    pub output_path: Option<PathBuf>,
    #[serde(skip)]
    pub dir: PathBuf,
}

pub type JobMap = Arc<Mutex<HashMap<String, Job>>>;

/// Shared application state handed to every request handler.
#[derive(Clone)]
pub struct AppState {
    pub jobs: JobMap,
    pub data_dir: PathBuf,
    /// How long finished/abandoned files live before cleanup deletes them.
    pub retention_secs: i64,
}

impl AppState {
    pub fn new(data_dir: PathBuf, retention_secs: i64) -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
            data_dir,
            retention_secs,
        }
    }

    pub fn insert_job(&self, job: Job) {
        if let Ok(mut map) = self.jobs.lock() {
            map.insert(job.id.clone(), job);
        }
    }

    pub fn get_job(&self, id: &str) -> Option<Job> {
        self.jobs.lock().ok().and_then(|m| m.get(id).cloned())
    }

    /// Apply a mutation to a job in place, if it still exists.
    pub fn update_job<F: FnOnce(&mut Job)>(&self, id: &str, f: F) {
        if let Ok(mut map) = self.jobs.lock() {
            if let Some(job) = map.get_mut(id) {
                f(job);
            }
        }
    }

    /// Jobs sorted newest-first.
    pub fn list_jobs(&self) -> Vec<Job> {
        let mut jobs: Vec<Job> = self
            .jobs
            .lock()
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default();
        jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        jobs
    }

    pub fn remove_job(&self, id: &str) -> Option<Job> {
        self.jobs.lock().ok().and_then(|mut m| m.remove(id))
    }

    /// Directory that holds all files for a given job.
    pub fn job_dir(&self, id: &str) -> PathBuf {
        self.data_dir.join(id)
    }
}
