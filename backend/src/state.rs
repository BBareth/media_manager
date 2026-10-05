use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::Utc;
use serde::Serialize;
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};
use tokio::task::AbortHandle;

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
    /// Accepted, waiting for a free slot.
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
    /// The task doing the work. Deleting a job aborts it, which drops the child process handle,
    /// and every child is spawned with `kill_on_drop` — so deleting a running job stops ffmpeg or
    /// yt-dlp instead of leaving it to burn CPU into a directory that no longer exists.
    #[serde(skip)]
    task: Option<AbortHandle>,
}

impl Job {
    pub fn new(id: String, kind: JobKind, title: String, detail: String, dir: PathBuf) -> Self {
        Self {
            id,
            kind,
            status: JobStatus::Queued,
            progress: 0.0,
            title,
            detail,
            output_name: None,
            output_size: None,
            error: None,
            created_at: Utc::now().timestamp(),
            output_path: None,
            dir,
            task: None,
        }
    }
}

/// How many jobs of each sort may run at once. The rest wait as `Queued`.
#[derive(Clone, Copy, Serialize)]
pub struct Limits {
    pub max_encodes: usize,
    pub max_downloads: usize,
}

/// Shared application state handed to every request handler.
#[derive(Clone)]
pub struct AppState {
    jobs: Arc<Mutex<HashMap<String, Job>>>,
    /// Bumped on every change to the job map; `/api/events` waits on it.
    changes: Arc<watch::Sender<u64>>,
    encode_slots: Arc<Semaphore>,
    download_slots: Arc<Semaphore>,
    pub limits: Limits,
    pub data_dir: PathBuf,
    /// How long finished/abandoned files live before cleanup deletes them.
    pub retention_secs: i64,
}

impl AppState {
    pub fn new(data_dir: PathBuf, retention_secs: i64, limits: Limits) -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
            changes: Arc::new(watch::Sender::new(0)),
            encode_slots: Arc::new(Semaphore::new(limits.max_encodes)),
            download_slots: Arc::new(Semaphore::new(limits.max_downloads)),
            limits,
            data_dir,
            retention_secs,
        }
    }

    /// Take the lock, recovering it if a previous holder panicked.
    ///
    /// Every accessor here used to swallow a poisoned lock with `if let Ok(..)` or `.ok()`, which
    /// turns one panic anywhere in one job into a process that silently stops recording progress
    /// for *all* jobs, forever: every update becomes a no-op, the UI shows 0 % until the retention
    /// sweep removes the row, and nothing is logged. The map holds plain data with no invariant a
    /// panic could have broken half-way, so carrying on with it is both safe and the only useful
    /// option — an unhelpful process is worse than a loud one.
    fn jobs(&self) -> MutexGuard<'_, HashMap<String, Job>> {
        self.jobs.lock().unwrap_or_else(|poisoned| {
            tracing::error!("job map lock was poisoned by an earlier panic; continuing with it");
            poisoned.into_inner()
        })
    }

    fn changed(&self) {
        self.changes.send_modify(|v| *v = v.wrapping_add(1));
    }

    /// A receiver that wakes whenever any job is added, changed or removed.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub fn insert_job(&self, job: Job) {
        self.jobs().insert(job.id.clone(), job);
        self.changed();
    }

    /// Run `work` in the background as the body of job `id`, keeping a handle so that deleting
    /// the job can stop it.
    pub fn spawn_job<F>(&self, id: &str, work: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let handle = tokio::spawn(work).abort_handle();
        match self.jobs().get_mut(id) {
            Some(job) => job.task = Some(handle),
            // Deleted between insert and spawn: nobody is waiting for the result.
            None => handle.abort(),
        }
    }

    /// Wait for a free slot for this kind of work. The job shows as `Queued` until then.
    ///
    /// Running everything at once does not finish sooner: x264 already uses every core, so two
    /// 20-second 1080p encodes run side by side both land at ~20.5 s on four cores, where one
    /// after the other delivers the first at ~11 s and the second at ~22 s. Unbounded, a batch
    /// of ten uploads is ten encoders fighting over the same cores and memory, with every result
    /// arriving at the very end.
    pub async fn acquire_slot(&self, kind: JobKind) -> OwnedSemaphorePermit {
        let slots = match kind {
            JobKind::Download => &self.download_slots,
            JobKind::Transcode | JobKind::Compress => &self.encode_slots,
        };
        slots
            .clone()
            .acquire_owned()
            .await
            .expect("job slots are never closed")
    }

    pub fn get_job(&self, id: &str) -> Option<Job> {
        self.jobs().get(id).cloned()
    }

    pub fn is_tracked(&self, id: &str) -> bool {
        self.jobs().contains_key(id)
    }

    /// Apply a mutation to a job in place, if it still exists.
    pub fn update_job<F: FnOnce(&mut Job)>(&self, id: &str, f: F) {
        let found = match self.jobs().get_mut(id) {
            Some(job) => {
                f(job);
                true
            }
            None => false,
        };
        if found {
            self.changed();
        }
    }

    pub fn set_running(&self, id: &str) {
        self.update_job(id, |j| j.status = JobStatus::Running);
    }

    /// Move the progress bar forward. Never backwards: tools report out of order, and a retry
    /// that restarts its own count should not make the bar jump back.
    pub fn set_progress(&self, id: &str, pct: f32) {
        let pct = pct.clamp(0.0, 100.0);
        let moved = match self.jobs().get_mut(id) {
            Some(job) if pct > job.progress => {
                job.progress = pct;
                true
            }
            _ => false,
        };
        if moved {
            self.changed();
        }
    }

    pub fn append_detail(&self, id: &str, note: &str) {
        self.update_job(id, |j| j.detail = format!("{} · {note}", j.detail));
    }

    pub fn fail(&self, id: &str, msg: impl Into<String>) {
        let msg = msg.into();
        self.update_job(id, |j| {
            j.status = JobStatus::Failed;
            j.error = Some(msg);
        });
    }

    pub fn complete(&self, id: &str, output: PathBuf, output_name: String) {
        let size = std::fs::metadata(&output).ok().map(|m| m.len());
        self.update_job(id, |j| {
            j.status = JobStatus::Completed;
            j.progress = 100.0;
            j.output_path = Some(output);
            j.output_name = Some(output_name);
            j.output_size = size;
        });
    }

    /// Jobs sorted newest-first.
    pub fn list_jobs(&self) -> Vec<Job> {
        let mut jobs: Vec<Job> = self.jobs().values().cloned().collect();
        jobs.sort_by_key(|j| std::cmp::Reverse(j.created_at));
        jobs
    }

    /// IDs of jobs created before `cutoff` (unix seconds).
    pub fn created_before(&self, cutoff: i64) -> Vec<String> {
        self.jobs()
            .values()
            .filter(|j| j.created_at < cutoff)
            .map(|j| j.id.clone())
            .collect()
    }

    /// Forget a job and stop its work if it is still running. Its files are the caller's to
    /// delete.
    pub fn remove_job(&self, id: &str) -> Option<Job> {
        let job = self.jobs().remove(id)?;
        if let Some(task) = &job.task {
            task.abort();
        }
        self.changed();
        Some(job)
    }

    /// Directory that holds all files for a given job.
    pub fn job_dir(&self, id: &str) -> PathBuf {
        self.data_dir.join(id)
    }
}
