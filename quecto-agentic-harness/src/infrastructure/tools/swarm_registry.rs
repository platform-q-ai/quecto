use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use super::{ActiveExecutions, JobRegistry, JobState};

/// Retain this many finished artifact directories per execution registry.
pub(crate) const MAX_RETAINED_ARTIFACT_DIRS: usize = 32;

/// Only this registry can know which of its executions are live. The opaque
/// owner prefix excludes other registries, including those in other processes.
/// Deletes the oldest owned artifact directories once the retention ceiling is
/// passed. Directories belonging to a job that has not finished are never
/// removed, so a running program cannot have its output deleted underneath it.
pub(crate) fn prune_artifact_dirs(
    workspace: &Path,
    jobs: &JobRegistry,
    active: &ActiveExecutions,
    owner: &str,
) {
    let root = workspace.join(".quecto/swarm");
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
    let prefix = format!("py_{owner}_");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return;
    };
    let mut dirs: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
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

type RegisteredJobs = (std::path::PathBuf, String, std::sync::Weak<Mutex<Jobs>>);
static CONTEXT_JOBS: std::sync::Mutex<Vec<RegisteredJobs>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn register_context_jobs(
    context: &super::super::swarm_bridge::SwarmContext,
    jobs: &JobRegistry,
) {
    let mut registered = CONTEXT_JOBS.lock().unwrap();
    registered.retain(|(_, _, weak)| weak.strong_count() > 0);
    registered.push((
        context.checkout.clone(),
        context.member.clone(),
        Arc::downgrade(jobs),
    ));
}

pub(crate) fn cancel_context_jobs(context: &super::super::swarm_bridge::SwarmContext) {
    let jobs: Vec<_> = CONTEXT_JOBS
        .lock()
        .unwrap()
        .iter()
        .filter(|(checkout, member, _)| checkout == &context.checkout && member == &context.member)
        .filter_map(|(_, _, weak)| weak.upgrade())
        .collect();
    for registry in jobs {
        super::cancel_jobs(&registry);
    }
}

#[derive(Default)]
pub(crate) struct Jobs {
    pub(crate) stopped: bool,
    entries: HashMap<String, Arc<Mutex<JobState>>>,
}
impl std::ops::Deref for Jobs {
    type Target = HashMap<String, Arc<Mutex<JobState>>>;
    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}
impl std::ops::DerefMut for Jobs {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.entries
    }
}

/// Foreground children share terminal cancellation with background jobs, but
/// disappear from the registry when the invocation (including errors) ends.
pub(crate) struct ForegroundRegistration(JobRegistry, String);
impl ForegroundRegistration {
    pub(crate) fn new(
        jobs: &JobRegistry,
        id: &str,
        state: Arc<Mutex<JobState>>,
    ) -> Result<Self, crate::domain::error::DomainError> {
        let mut registry = jobs.lock().unwrap();
        if registry.stopped {
            return Err(crate::domain::error::DomainError::Tool(
                "swarm execution registry stopped".into(),
            ));
        }
        registry.insert(id.to_owned(), state);
        Ok(Self(jobs.clone(), id.to_owned()))
    }
}
impl Drop for ForegroundRegistration {
    fn drop(&mut self) {
        if let Some(state) = self.0.lock().unwrap().remove(&self.1) {
            let mut state = state.lock().unwrap();
            state.cancel_requested = true;
            if let Some(pid) = state.pid {
                super::terminate_member(pid);
            }
        }
    }
}

pub(crate) fn cancel_jobs(jobs: &JobRegistry) {
    if let Ok(mut jobs) = jobs.lock() {
        jobs.stopped = true;
        for job in jobs.values() {
            if let Ok(mut job) = job.lock() {
                if !is_terminal(&job.status) {
                    job.cancel_requested = true;
                    if let Some(pid) = job.pid {
                        super::terminate_member(pid);
                    }
                }
            }
        }
    }
}
