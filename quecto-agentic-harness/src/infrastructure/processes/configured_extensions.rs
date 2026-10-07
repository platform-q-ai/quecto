//! The configured extensions one agent runs (#2446): launched once its
//! socket listens, each in a process group of its own owned by the
//! [`OwnedChildSupervisor`] (so the agent's exit ends it, and on Linux its
//! parent-death signal ends it when the agent is killed outright), with
//! stdin closed and its output appended to `<state_dir>/extension.log`.
//! An exit is restarted by the domain's restart budget; codes 2 and 3 are
//! final. The board keeps each extension's state for `get_state`, knows
//! which socket connections are an extension's own (by the process group
//! of the peer), and says when the start-up wait is over.
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::domain::agents::entities::configured_extensions::{
    AfterExit, AgentExtensions, ExtensionExit, ExtensionSpec, ExtensionState, PlaceholderValues,
    REGISTRATION_WAIT, RestartBudget, StopReason,
};

use super::owned_child_supervisor::{
    ChildExit, ChildHandleId, OwnedChildSupervisor, ProcessGroup, ProtocolOutcome,
    TerminationBudget,
};

/// An extension's bounded end when its agent exits: TERM, then KILL.
const SHUTDOWN_BUDGET: TerminationBudget = TerminationBudget {
    exit_after_ack: Duration::ZERO,
    term_grace: Duration::from_secs(5),
    kill_grace: Duration::from_secs(2),
};

/// One extension, its placeholders expanded, ready to launch.
#[derive(Debug, Clone)]
pub struct ExtensionLaunch {
    name: String,
    /// Its placeholders expanded; an expansion error fails its launch.
    spec: Result<ExtensionSpec, String>,
    state_dir: PathBuf,
    log: PathBuf,
    /// Remove `state_dir` once the instance has ended (a sub-agent's).
    discard_state: bool,
}

/// Expand `extensions` for an agent listening on `socket`: each instance's
/// state directory is `<base_dir>/extensions/<name>/<agent_id>`.
pub fn plan(extensions: &AgentExtensions, socket: &Path, base_dir: &Path) -> Vec<ExtensionLaunch> {
    extensions
        .specs
        .iter()
        .map(|spec| {
            let state_dir = base_dir
                .join("extensions")
                .join(&spec.name)
                .join(&extensions.agent_id);
            let values = PlaceholderValues {
                socket: &socket.to_string_lossy(),
                agent_id: &extensions.agent_id,
                state_dir: &state_dir.to_string_lossy(),
            };
            ExtensionLaunch {
                name: spec.name.clone(),
                spec: spec.expanded(&values),
                log: state_dir.join("extension.log"),
                state_dir,
                discard_state: extensions.discards_state(),
            }
        })
        .collect()
}

struct Entry {
    name: String,
    log: String,
    state: ExtensionState,
    /// The running instance and its process group's id (its leader's pid).
    running: Option<(ChildHandleId, u32)>,
    /// A launch is in flight: its process may exist before `running` says so.
    launching: bool,
}

struct Board {
    entries: Mutex<Vec<Entry>>,
    /// Set once, when the agent exits: every supervisor ends its instance.
    stopping: tokio::sync::watch::Sender<bool>,
    revision: tokio::sync::watch::Sender<u64>,
}

impl Board {
    fn lock(&self) -> MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Change entry `index` and announce it.
    fn update(&self, index: usize, change: impl FnOnce(&mut Entry)) {
        let mut entries = self.lock();
        let entry = entries.get_mut(index).expect("an entry per launch");
        let (state, running) = (entry.state.clone(), entry.running);
        change(entry);
        let moved = entry.state != state;
        if let Some(warning) = entry
            .state
            .warning(&entry.name, &entry.log)
            .filter(|_| moved)
        {
            eprintln!("warning: {warning}");
        }
        let changed = moved || entry.running != running;
        drop(entries);
        if changed {
            self.revision.send_modify(|revision| *revision += 1);
        }
    }

    /// Move entry `index` from `from` to `to`; any other state stays.
    fn advance(&self, index: usize, from: &ExtensionState, to: ExtensionState) {
        self.update(index, |entry| {
            if entry.state == *from {
                entry.state = to;
            }
        });
    }
}

/// The configured extensions of one agent.
pub struct ConfiguredExtensions {
    board: Arc<Board>,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    /// Held for a whole shutdown, so a second call waits for the first.
    ending: tokio::sync::Mutex<()>,
    /// State directories removed once the instances have ended (a
    /// sub-agent's; see [`AgentExtensions::discards_state`]).
    discarded: Vec<PathBuf>,
}

impl std::fmt::Debug for ConfiguredExtensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfiguredExtensions")
            .finish_non_exhaustive()
    }
}

impl ConfiguredExtensions {
    /// Launch every planned extension, each supervised by a task on the
    /// current runtime.
    pub fn launch(launches: Vec<ExtensionLaunch>, supervisor: Arc<OwnedChildSupervisor>) -> Self {
        let board = Arc::new(Board {
            entries: Mutex::new(
                launches
                    .iter()
                    .map(|launch| Entry {
                        name: launch.name.clone(),
                        log: launch.log.display().to_string(),
                        state: ExtensionState::Starting,
                        running: None,
                        launching: false,
                    })
                    .collect(),
            ),
            stopping: tokio::sync::watch::channel(false).0,
            revision: tokio::sync::watch::channel(0).0,
        });
        let discarded = launches
            .iter()
            .filter(|launch| launch.discard_state)
            .map(|launch| launch.state_dir.clone())
            .collect();
        let tasks = launches
            .into_iter()
            .enumerate()
            .map(|(index, launch)| {
                tokio::spawn(supervise(
                    Arc::clone(&board),
                    Arc::clone(&supervisor),
                    index,
                    launch,
                ))
            })
            .collect();
        Self {
            board,
            tasks: Mutex::new(tasks),
            ending: tokio::sync::Mutex::new(()),
            discarded,
        }
    }

    /// The extension whose process group `peer_pid` belongs to, as a
    /// claim on its connection; `None` for any other peer.
    pub fn claim(&self, peer_pid: Option<i32>) -> Option<ExtensionClaim> {
        let group = process_group_of(peer_pid?)?;
        let index = self.board.lock().iter().position(|entry| {
            entry
                .running
                .is_some_and(|(_, leader)| i32::try_from(leader) == Ok(group))
        })?;
        Some(ExtensionClaim {
            board: Arc::clone(&self.board),
            index,
        })
    }

    /// Resolves once no launch is in flight (bounded): a connection that
    /// arrived before its extension's launch was recorded can then be
    /// claimed, closing that race.
    pub async fn launched(&self) {
        let mut changes = self.board.revision.subscribe();
        let launched = || self.board.lock().iter().all(|entry| !entry.launching);
        let wait = async {
            while !launched() {
                if changes.changed().await.is_err() {
                    return;
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(2), wait).await;
    }

    /// Resolves once every extension has registered its tools, ran out of
    /// [`REGISTRATION_WAIT`], or will not run.
    pub async fn settled(&self) {
        let mut changes = self.board.revision.subscribe();
        let settled = || self.board.lock().iter().all(|entry| entry.state.settled());
        while !settled() {
            if changes.changed().await.is_err() {
                return;
            }
        }
    }

    /// One warning per extension in trouble, for `startupWarnings`.
    pub fn warnings(&self) -> Vec<String> {
        self.board
            .lock()
            .iter()
            .filter_map(|entry| entry.state.warning(&entry.name, &entry.log))
            .collect()
    }

    /// Advances whenever any extension's state changes.
    pub fn revision(&self) -> u64 {
        *self.board.revision.borrow()
    }

    /// The agent is exiting: nothing is restarted, and every supervisor
    /// ends its instance's whole process group (TERM, then KILL), one that
    /// is mid-launch included, before this returns (bounded); then a
    /// sub-agent's state directories go.
    pub async fn shutdown(&self) {
        let _ending = self.ending.lock().await;
        self.board.stopping.send_replace(true);
        let tasks = std::mem::take(&mut *self.tasks.lock().unwrap_or_else(|e| e.into_inner()));
        let bound =
            SHUTDOWN_BUDGET.term_grace + SHUTDOWN_BUDGET.kill_grace + Duration::from_secs(1);
        if tokio::time::timeout(bound, futures::future::join_all(tasks))
            .await
            .is_err()
        {
            tracing::warn!("a configured extension did not end within {bound:?}");
        }
        for state_dir in &self.discarded {
            if let Err(error) = std::fs::remove_dir_all(state_dir) {
                tracing::debug!(dir = %state_dir.display(), "state directory not removed: {error}");
            }
        }
    }
}

/// A socket connection that is a configured extension's own.
pub struct ExtensionClaim {
    board: Arc<Board>,
    index: usize,
}

impl std::fmt::Debug for ExtensionClaim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ExtensionClaim").field(&self.name()).finish()
    }
}

impl ExtensionClaim {
    pub fn name(&self) -> String {
        self.board.lock()[self.index].name.clone()
    }

    /// Its `register_tools` succeeded.
    pub fn tools_registered(&self) {
        self.board
            .update(self.index, |entry| entry.state = ExtensionState::Running);
    }
}

impl Drop for ExtensionClaim {
    /// Its connection closed: a running extension without one has no tools.
    fn drop(&mut self) {
        let running = ExtensionState::Running;
        self.board
            .advance(self.index, &running, ExtensionState::Disconnected);
    }
}

/// Run one extension until it stops for good or its agent exits; on the
/// agent's exit, end the running instance.
async fn supervise(
    board: Arc<Board>,
    supervisor: Arc<OwnedChildSupervisor>,
    index: usize,
    launch: ExtensionLaunch,
) {
    let mut budget = RestartBudget::default();
    let mut stop = board.stopping.subscribe();
    while !*stop.borrow() {
        board.update(index, |entry| entry.launching = true);
        let spawned = match start(&supervisor, &launch).await {
            Ok(spawned) => spawned,
            Err(error) => {
                board.update(index, |entry| {
                    entry.launching = false;
                    entry.state = ExtensionState::Stopped {
                        exit: None,
                        reason: StopReason::LaunchFailed(error),
                    }
                });
                return;
            }
        };
        board.update(index, |entry| {
            entry.running = Some((spawned.handle, spawned.display_pid.0));
            entry.launching = false;
            entry.state = ExtensionState::Starting;
        });
        let exit_wait = supervisor.wait_exit(spawned.handle);
        tokio::pin!(exit_wait);
        let overdue = tokio::time::sleep(REGISTRATION_WAIT);
        tokio::pin!(overdue);
        let mut waiting_for_tools = true;
        let exit = loop {
            tokio::select! {
                exit = &mut exit_wait => break exit,
                () = &mut overdue, if waiting_for_tools => {
                    waiting_for_tools = false;
                    board.advance(index, &ExtensionState::Starting, ExtensionState::Unregistered);
                }
                () = stopped(&mut stop) => {
                    let negative = async { ProtocolOutcome::Negative("its agent is exiting".into()) };
                    let outcome = supervisor.terminate(spawned.handle, negative, SHUTDOWN_BUDGET).await;
                    tracing::info!(extension = %launch.name, ?outcome, "configured extension ended");
                    supervisor.retire(spawned.handle);
                    return;
                }
            }
        };
        let exit = match exit {
            Some(ChildExit::Code(code)) => ExtensionExit::Code(code),
            Some(ChildExit::Signal(signal)) => ExtensionExit::Signal(signal),
            Some(ChildExit::Unobservable(_)) | None => ExtensionExit::Unobservable,
        };
        supervisor.retire(spawned.handle);
        let after = budget.after_exit(&exit, Instant::now());
        board.update(index, |entry| {
            entry.running = None;
            entry.state = match after.clone() {
                AfterExit::Restart { restart, .. } => ExtensionState::Restarting {
                    exit: exit.clone(),
                    restart,
                },
                AfterExit::Stop(reason) => ExtensionState::Stopped {
                    exit: Some(exit.clone()),
                    reason,
                },
            }
        });
        let delay = match after {
            AfterExit::Restart { delay, .. } => delay,
            AfterExit::Stop(_) => return,
        };
        tokio::select! {
            () = tokio::time::sleep(delay) => {}
            () = stopped(&mut stop) => return,
        }
    }
}

/// Resolves once the agent is exiting (a closed channel counts).
async fn stopped(stop: &mut tokio::sync::watch::Receiver<bool>) {
    let _ = stop.wait_for(|stopping| *stopping).await;
}

/// Spawn one instance: its state directory made, stdin closed, its output
/// appended to its log, its own process group, armed to die with the agent.
async fn start(
    supervisor: &Arc<OwnedChildSupervisor>,
    launch: &ExtensionLaunch,
) -> Result<super::owned_child_supervisor::SpawnedChild, String> {
    let spec = launch.spec.as_ref().map_err(Clone::clone)?;
    let describe = |what: &str, path: &Path, error: std::io::Error| {
        format!("{what} {}: {error}", path.display())
    };
    // Private: an instance's state (a browser profile, say) is its owner's.
    let mut directory = std::fs::DirBuilder::new();
    let mut file = std::fs::OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        directory.mode(0o700);
        file.mode(0o600);
    }
    directory
        .recursive(true)
        .create(&launch.state_dir)
        .map_err(|error| describe("cannot create", &launch.state_dir, error))?;
    let log = file
        .create(true)
        .append(true)
        .open(&launch.log)
        .map_err(|error| describe("cannot open", &launch.log, error))?;
    let stderr = log
        .try_clone()
        .map_err(|error| describe("cannot open", &launch.log, error))?;
    // One made before (by an earlier run, or by hand) is made private too.
    #[cfg(unix)]
    for (path, mode) in [(&launch.state_dir, 0o700), (&launch.log, 0o600)] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(|error| describe("cannot make private", path, error))?;
    }
    let mut command = tokio::process::Command::new(&spec.command);
    command
        .args(&spec.args)
        .envs(&spec.env)
        .stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(stderr);
    super::parent_death_signal::arm(&mut command, std::process::id());
    supervisor
        .spawn(command, ProcessGroup::Own)
        .await
        .map_err(|error| format!("{}: {error}", spec.command))
}

/// The process group `pid` belongs to, if it can be read.
fn process_group_of(pid: i32) -> Option<i32> {
    // SAFETY: getpgid(2) only reads the group id of a process.
    let group = unsafe { libc::getpgid(pid) };
    (group > 0).then_some(group)
}

#[cfg(test)]
#[path = "configured_extensions_tests.rs"]
mod tests;
