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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::domain::agents::configured_extensions::{
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
    /// Bumped per launch, so a stale registration deadline is ignored.
    launch: u64,
}

struct Board {
    entries: Mutex<Vec<Entry>>,
    stopping: AtomicBool,
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
}

/// The configured extensions of one agent.
pub struct ConfiguredExtensions {
    board: Arc<Board>,
    supervisor: Arc<OwnedChildSupervisor>,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
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
                        launch: 0,
                    })
                    .collect(),
            ),
            stopping: AtomicBool::new(false),
            revision: tokio::sync::watch::channel(0).0,
        });
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
            supervisor,
            tasks: Mutex::new(tasks),
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

    /// The agent is exiting: nothing is restarted, and every running
    /// extension is ended (TERM to its group, then KILL).
    pub async fn shutdown(&self) {
        self.board.stopping.store(true, Ordering::SeqCst);
        let running: Vec<ChildHandleId> = self
            .board
            .lock()
            .iter()
            .filter_map(|entry| entry.running.map(|(handle, _)| handle))
            .collect();
        futures::future::join_all(running.into_iter().map(|handle| {
            self.supervisor.terminate(
                handle,
                async { ProtocolOutcome::Negative("its agent is exiting".into()) },
                SHUTDOWN_BUDGET,
            )
        }))
        .await;
        for task in self
            .tasks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
        {
            task.abort();
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
        self.board.update(self.index, |entry| {
            if entry.state == ExtensionState::Running {
                entry.state = ExtensionState::Unregistered;
            }
        });
    }
}

/// Run one extension until it stops for good or its agent exits.
async fn supervise(
    board: Arc<Board>,
    supervisor: Arc<OwnedChildSupervisor>,
    index: usize,
    launch: ExtensionLaunch,
) {
    let mut budget = RestartBudget::default();
    while !board.stopping.load(Ordering::SeqCst) {
        let spawned = match start(&supervisor, &launch).await {
            Ok(spawned) => spawned,
            Err(error) => {
                board.update(index, |entry| {
                    entry.state = ExtensionState::Stopped {
                        exit: None,
                        reason: StopReason::LaunchFailed(error),
                    }
                });
                return;
            }
        };
        let mut launched = 0;
        board.update(index, |entry| {
            entry.launch += 1;
            launched = entry.launch;
            entry.running = Some((spawned.handle, spawned.display_pid.0));
            entry.state = ExtensionState::Starting;
        });
        if board.stopping.load(Ordering::SeqCst) {
            // Launched while its agent began exiting: ended at once.
            let negative = async { ProtocolOutcome::Negative("its agent is exiting".into()) };
            supervisor
                .terminate(spawned.handle, negative, SHUTDOWN_BUDGET)
                .await;
            return;
        }
        let overdue = Arc::clone(&board);
        tokio::spawn(async move {
            tokio::time::sleep(REGISTRATION_WAIT).await;
            overdue.update(index, |entry| {
                if entry.launch == launched && entry.state == ExtensionState::Starting {
                    entry.state = ExtensionState::Unregistered;
                }
            });
        });
        let exit = match supervisor.wait_exit(spawned.handle).await {
            Some(ChildExit::Code(code)) => ExtensionExit::Code(code),
            Some(ChildExit::Signal(signal)) => ExtensionExit::Signal(signal),
            Some(ChildExit::Unobservable(_)) | None => ExtensionExit::Unobservable,
        };
        supervisor.retire(spawned.handle);
        if board.stopping.load(Ordering::SeqCst) {
            return;
        }
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
        match after {
            AfterExit::Restart { delay, .. } => tokio::time::sleep(delay).await,
            AfterExit::Stop(_) => return,
        }
    }
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
