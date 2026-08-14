use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use super::{ActiveExecutions, JobRegistry, JobState};

/// forever.
pub(crate) const MAX_RETAINED_ARTIFACT_DIRS: usize = 32;

/// Deletes the oldest artifact directories once the retention ceiling is
/// passed. Directories belonging to a job that has not finished are never
/// removed, so a running program cannot have its output deleted underneath it.
pub(crate) fn prune_artifact_dirs(workspace: &Path, jobs: &JobRegistry, active: &ActiveExecutions) {
    let root = workspace.join(".quecto/python_lab");
    let mut live: Vec<String> = active
        .lock()
        .map(|set| set.iter().cloned().collect())
        .unwrap_or_default();
    live.extend(
        jobs.lock()
            .map(|registry| {
                registry
                    .values()
                    .filter_map(|job| {
                        let j = job.lock().ok()?;
                        (!is_terminal(&j.status)).then(|| j.execution_id.clone())
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    let mut dirs: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !live.iter().any(|id| *id == e.file_name().to_string_lossy()))
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((modified, e.path()))
        })
        .collect();
    if dirs.len() <= MAX_RETAINED_ARTIFACT_DIRS {
        return;
    }
    dirs.sort_unstable_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    for (_, path) in dirs.into_iter().skip(MAX_RETAINED_ARTIFACT_DIRS) {
        let _ = std::fs::remove_dir_all(path);
    }
}

/// A job is terminal once it has been reaped and its result published.
pub(crate) fn is_terminal(status: &str) -> bool {
    !matches!(status, "running" | "cancelling")
}

/// Completed jobs are kept so their results stay retrievable, but not forever:
/// each retained job holds its full result JSON. Once the retention ceiling is
/// reached the oldest finished jobs are dropped. Live jobs are never evicted.
pub(crate) const MAX_RETAINED_JOBS: usize = 32;

pub(crate) fn evict_finished_jobs(registry: &mut HashMap<String, Arc<Mutex<JobState>>>) {
    if registry.len() < MAX_RETAINED_JOBS {
        return;
    }
    let mut finished: Vec<(u128, String)> = registry
        .iter()
        .filter_map(|(id, job)| {
            let j = job.lock().ok()?;
            is_terminal(&j.status).then(|| (j.completed_ms.unwrap_or(j.started_ms), id.clone()))
        })
        .collect();
    finished.sort_unstable();
    for (_, id) in finished
        .into_iter()
        .take((registry.len() + 1).saturating_sub(MAX_RETAINED_JOBS))
    {
        registry.remove(&id);
    }
}
