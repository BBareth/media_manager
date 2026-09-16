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
        sweep(&state);
        tokio::time::sleep(SWEEP_INTERVAL).await;
    }
}

fn sweep(state: &AppState) {
    let now = Utc::now().timestamp();
    let cutoff = now - state.retention_secs;

    // 1. Expire tracked jobs by their creation time.
    let expired: Vec<String> = {
        let Ok(map) = state.jobs.lock() else { return };
        map.values()
            .filter(|j| j.created_at < cutoff)
            .map(|j| j.id.clone())
            .collect()
    };
    for id in &expired {
        if let Some(job) = state.remove_job(id) {
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
            if state.get_job(name).is_some() {
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
