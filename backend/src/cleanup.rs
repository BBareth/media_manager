use std::fs;
use std::time::{Duration, UNIX_EPOCH};

use chrono::Utc;

use crate::state::AppState;

/// How often the cleanup sweep runs.
const SWEEP_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Periodically delete jobs and on-disk files older than the retention window.
/// Runs forever; spawn it once at startup.
pub async fn run_cleanup(state: AppState) {
    // First sweep shortly after boot to clear anything left from a previous run.
    tokio::time::sleep(Duration::from_secs(10)).await;
    loop {
        // Deleting a day's worth of media is slow, blocking filesystem work; keep it off the
        // threads that serve requests.
        let st = state.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || sweep(&st)).await {
            tracing::error!("cleanup sweep panicked: {e}");
        }
        tokio::time::sleep(SWEEP_INTERVAL).await;
    }
}

fn sweep(state: &AppState) {
    let cutoff = Utc::now().timestamp() - state.retention_secs;

    // 1. Expire tracked jobs by their creation time. Removing one that is somehow still running
    //    stops it, too.
    for id in state.created_before(cutoff) {
        if let Some(job) = state.remove_job(&id) {
            let _ = fs::remove_dir_all(&job.dir);
            tracing::info!(job = %id, "cleanup: removed expired job");
        }
    }

    // 2. Remove orphaned directories on disk (e.g. left by a crash) whose
    //    modification time is older than the retention window.
    let cutoff_system = UNIX_EPOCH + Duration::from_secs(cutoff.max(0) as u64);
    let Ok(entries) = fs::read_dir(&state.data_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // Skip directories still tracked as active jobs.
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if state.is_tracked(name) {
                continue;
            }
        }
        let too_old = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|m| m < cutoff_system)
            .unwrap_or(false);
        if too_old {
            let _ = fs::remove_dir_all(&path);
            tracing::info!(dir = ?path, "cleanup: removed orphaned directory");
        }
    }
}
