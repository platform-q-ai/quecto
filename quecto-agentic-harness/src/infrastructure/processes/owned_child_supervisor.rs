//! The one owner of every directly spawned child process (#1935).
//!
//! Every `tokio::process::Child` this harness creates — local subagent
//! launches and per-connection proxy bridge processes — is adopted here the
//! moment it is spawned and never leaves. Callers get an opaque
//! [`ChildHandleId`]; they never hold, learn or pass a pid. The supervisor:
//!
//! - serialises **wait/reap**: one reap task per handle polls the child's
//!   exit under the slot lock and marks it reaped in the same critical
//!   section, so no signal can race a freed pid;
//! - allows **TERM → bounded wait → KILL** only while the concrete unreaped
//!   handle (and its own process group, where one was created) is retained,
//!   and only *after* the caller's protocol attempt came back negative or
//!   its deadline passed. The protocol attempt is always awaited first;
//! - sends TERM at most once and KILL at most once per handle, whatever the
//!   number or fate of concurrent or cancelled termination calls;
//! - has **no fallback at all** for anything it does not hold: a
//!   script/container member without a local handle, a copied, restored or
//!   fixture pid, gets [`TerminationOutcome::NoRetainedHandle`] and no signal.
//!
//! The supervisor owns a small runtime of its own on which every process is
//! spawned and reaped. Its lifetime is therefore independent of whichever
//! runtime happened to be current at launch: a launcher that builds a
//! runtime per call (tests, embedders) cannot orphan a reap, and a
//! `tokio::process::Child` never exists outside this module.
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::watch;

/// Opaque identity of one adopted child. Not a pid, not derivable from one.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChildHandleId(u64);

impl ChildHandleId {
    /// Test seam: name an id the supervisor never issued, to prove that an
    /// unknown handle gets no signal.
    #[cfg(any(test, feature = "test-support"))]
    pub const fn probe(raw: u64) -> Self {
        Self(raw)
    }
}

impl std::fmt::Debug for ChildHandleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ChildHandle#{}", self.0)
    }
}

/// Display-only pid handed back once at adoption for registry listings and
/// wire snapshots (retained until #1940). It grants nothing: the supervisor
/// never accepts it back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayPid(pub u32);

/// Whether the child was spawned into its own process group (`pgid == pid`),
/// in which case fallback signals address the whole group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessGroup {
    Inherited,
    Own,
}

/// How an adopted child ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildExit {
    Code(i32),
    Signal(i32),
    /// `wait` itself failed; the pid is still reaped as far as this harness
    /// is concerned (tokio never returns it again).
    Unobservable(String),
}

impl ChildExit {
    fn from_status(status: std::io::Result<std::process::ExitStatus>) -> Self {
        match status {
            Ok(status) => {
                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    if let Some(signal) = status.signal() {
                        return Self::Signal(signal);
                    }
                }
                Self::Code(status.code().unwrap_or(-1))
            }
            Err(error) => Self::Unobservable(error.to_string()),
        }
    }
}

/// Result of the caller's protocol attempt, in the supervisor's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolOutcome {
    /// The child acknowledged the shutdown; it is expected to exit by itself.
    Acknowledged,
    /// The child could not be reached, refused, or the attempt timed out.
    Negative(String),
}

/// Bounded waits of one termination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminationBudget {
    /// How long an acknowledged child gets to exit before the fallback.
    pub exit_after_ack: Duration,
    /// How long after TERM before KILL.
    pub term_grace: Duration,
    /// How long after KILL before giving up on observing the exit.
    pub kill_grace: Duration,
}

impl TerminationBudget {
    pub const DEFAULT: Self = Self {
        exit_after_ack: Duration::from_secs(10),
        term_grace: Duration::from_secs(2),
        kill_grace: Duration::from_secs(2),
    };
}

impl Default for TerminationBudget {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminationOutcome {
    /// No unreaped handle is retained for this id: nothing was signalled.
    NoRetainedHandle,
    /// The child had already exited before any step was needed.
    AlreadyExited(ChildExit),
    /// The child exited on its own after acknowledging the protocol.
    ExitedAfterProtocol(ChildExit),
    /// The protocol outcome was negative (or the exit deadline passed) and
    /// TERM produced the exit.
    ExitedAfterTerm { negative: String, exit: ChildExit },
    /// TERM did not suffice within its grace; KILL produced the exit.
    ExitedAfterKill { negative: String, exit: ChildExit },
    /// Even KILL did not yield an observed exit within the budget.
    StillRunning { negative: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SentSignal {
    Term,
    Kill,
}

struct Slot {
    pid: u32,
    group: ProcessGroup,
    reaped: bool,
    exit: watch::Sender<Option<ChildExit>>,
    term_sent: bool,
    kill_sent: bool,
    /// Negative protocol outcome that authorised the fallback, once.
    fallback_authorised_by: Option<String>,
    sent: Vec<SentSignal>,
}

/// What a caller gets back from [`OwnedChildSupervisor::spawn`]: the opaque
/// handle, the display pid, and any stdio pipes the command asked for. The
/// process itself stays with the supervisor.
pub struct SpawnedChild {
    pub handle: ChildHandleId,
    pub display_pid: DisplayPid,
    pub stdin: Option<tokio::process::ChildStdin>,
    pub stdout: Option<tokio::process::ChildStdout>,
    pub stderr: Option<tokio::process::ChildStderr>,
}

/// See the module docs. One instance per process, wired by composition.
/// How many retired handles keep their signal record for late observers.
const RETIRED_RECORDS: usize = 64;

/// What survives a retired slot: the signals that were sent, so a late
/// observer (a registry clone, a test) still learns what happened.
#[derive(Debug, Clone)]
struct RetiredRecord {
    id: ChildHandleId,
    sent: Vec<SentSignal>,
    fallback_authorised_by: Option<String>,
    exit: Option<ChildExit>,
}

pub struct OwnedChildSupervisor {
    slots: Mutex<HashMap<ChildHandleId, Slot>>,
    /// Bounded ring of retired handles' signal records.
    retired: Mutex<std::collections::VecDeque<RetiredRecord>>,
    next: AtomicU64,
    /// The supervisor's own runtime: every spawn and reap runs here.
    runtime: Mutex<Option<tokio::runtime::Runtime>>,
    handle: tokio::runtime::Handle,
    /// Test seam: record instead of dispatching real signals.
    #[cfg(any(test, feature = "test-support"))]
    dry_run: std::sync::atomic::AtomicBool,
}

impl Drop for OwnedChildSupervisor {
    fn drop(&mut self) {
        // Never block on the runtime's shutdown: the last owner may be an
        // async task. Unreaped children keep running; nothing signals them.
        if let Some(runtime) = self
            .runtime
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            runtime.shutdown_background();
        }
    }
}

impl std::fmt::Debug for OwnedChildSupervisor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedChildSupervisor")
            .field("handles", &self.handles())
            .finish_non_exhaustive()
    }
}

impl Default for OwnedChildSupervisor {
    fn default() -> Self {
        Self::new()
    }
}

static PROCESS_WIDE: std::sync::OnceLock<Arc<OwnedChildSupervisor>> = std::sync::OnceLock::new();

impl OwnedChildSupervisor {
    /// The process-wide supervisor every launcher in this process adopts
    /// into, created on first use. A launcher built without an explicit
    /// supervisor uses this one, so no reap task is ever abandoned with a
    /// throwaway instance.
    pub fn process_wide() -> Arc<Self> {
        PROCESS_WIDE.get_or_init(|| Arc::new(Self::new())).clone()
    }

    pub fn new() -> Self {
        // One worker thread drives the process signal driver, the reap
        // tasks and detached terminations for the supervisor's whole life.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .thread_name("quecto-child-supervisor")
            .build()
            .expect("owned child supervisor runtime");
        let handle = runtime.handle().clone();
        Self {
            slots: Mutex::new(HashMap::new()),
            retired: Mutex::new(std::collections::VecDeque::new()),
            next: AtomicU64::new(1),
            runtime: Mutex::new(Some(runtime)),
            handle,
            #[cfg(any(test, feature = "test-support"))]
            dry_run: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Spawn `command` on the supervisor's runtime and own the result. The
    /// only way a process enters this harness's ownership: no caller ever
    /// holds the `Child`. Any piped stdio is handed back for the caller to
    /// pump.
    pub async fn spawn(
        self: &Arc<Self>,
        mut command: tokio::process::Command,
        group: ProcessGroup,
    ) -> std::io::Result<SpawnedChild> {
        let supervisor = Arc::clone(self);
        self.handle
            .spawn(async move {
                let mut child = command.spawn()?;
                let stdin = child.stdin.take();
                let stdout = child.stdout.take();
                let stderr = child.stderr.take();
                let (handle, display_pid) = supervisor.adopt(child, group);
                Ok(SpawnedChild {
                    handle,
                    display_pid,
                    stdin,
                    stdout,
                    stderr,
                })
            })
            .await
            .expect("the supervisor runtime outlives every spawn")
    }

    /// Drain a child's piped stderr on the supervisor's runtime for the
    /// child's whole life — where the reap task lives, so no launcher
    /// runtime can close the pipe under the child — retaining a bounded tail
    /// for the launch failure report (#1937 review).
    pub fn retain_stderr_tail(
        &self,
        stderr: tokio::process::ChildStderr,
    ) -> super::child_stderr_tail::StderrTail {
        super::child_stderr_tail::StderrTail::pump(&self.handle, stderr)
    }

    /// Record signals instead of sending them (tests with fake pids).
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_dry_run(&self, dry_run: bool) {
        self.dry_run.store(dry_run, Ordering::SeqCst);
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<ChildHandleId, Slot>> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Take ownership of a child spawned on the supervisor's runtime (the
    /// reap task must live where the process's signal driver lives). Returns
    /// the opaque handle and the display pid.
    fn adopt(
        self: &Arc<Self>,
        mut child: tokio::process::Child,
        group: ProcessGroup,
    ) -> (ChildHandleId, DisplayPid) {
        let id = ChildHandleId(self.next.fetch_add(1, Ordering::SeqCst));
        let pid = child.id().unwrap_or(0);
        let (exit, _) = watch::channel(None);
        self.lock().insert(
            id,
            Slot {
                pid,
                group,
                reaped: false,
                exit,
                term_sent: false,
                kill_sent: false,
                fallback_authorised_by: None,
                sent: Vec::new(),
            },
        );
        let supervisor = Arc::clone(self);
        self.handle.spawn(async move {
            {
                let mut wait = Box::pin(child.wait());
                // Poll the wait under the slot lock and mark the slot reaped
                // in the same critical section, so a fallback signal can
                // never be dispatched to a pid the kernel has already freed.
                std::future::poll_fn(|cx| {
                    let mut slots = supervisor.lock();
                    let result = wait.as_mut().poll(cx);
                    if let std::task::Poll::Ready(status) = result {
                        let exit = ChildExit::from_status(status);
                        if let Some(slot) = slots.get_mut(&id) {
                            slot.reaped = true;
                            slot.exit.send_replace(Some(exit));
                        }
                        return std::task::Poll::Ready(());
                    }
                    std::task::Poll::Pending
                })
                .await;
            }
            // The handle itself is dropped here, after the reap: nothing can
            // observe it in between.
            drop(child);
        });
        (id, DisplayPid(pid))
    }

    /// Forget a reaped handle once its owner has observed the exit (the
    /// reaper after its cleanup, the proxy bridge after its termination), so
    /// the slot table does not grow by one entry per process ever spawned.
    /// An unreaped handle is never retired: `false`, nothing changes.
    pub fn retire(&self, id: ChildHandleId) -> bool {
        let mut slots = self.lock();
        let Some(slot) = slots.get(&id).filter(|slot| slot.reaped) else {
            return false;
        };
        let record = RetiredRecord {
            id,
            sent: slot.sent.clone(),
            fallback_authorised_by: slot.fallback_authorised_by.clone(),
            exit: slot.exit.borrow().clone(),
        };
        slots.remove(&id);
        drop(slots);
        let mut retired = self.retired.lock().unwrap_or_else(|e| e.into_inner());
        if retired.len() == RETIRED_RECORDS {
            retired.pop_front();
        }
        retired.push_back(record);
        true
    }

    /// Retire `id` as soon as its reap completes, for owners that do not
    /// wait for the exit themselves (a rolled-back launch, a proxy that
    /// outlived its termination budget). A no-op for unknown handles.
    pub fn retire_when_reaped(self: &Arc<Self>, id: ChildHandleId) {
        if self.retire(id) {
            return;
        }
        let Some(mut receiver) = self.exit_receiver(id) else {
            return;
        };
        let supervisor = Arc::clone(self);
        self.handle.spawn(async move {
            loop {
                if receiver.borrow().is_some() {
                    break;
                }
                if receiver.changed().await.is_err() {
                    break;
                }
            }
            supervisor.retire(id);
        });
    }

    /// The outcome for a handle that was retired while a termination was
    /// in flight: the recorded exit, named by the signals that were sent
    /// (by anyone), never a fabricated "still running". `acknowledged` is
    /// whether this caller's protocol attempt was accepted. `None` when the
    /// record has left the ring.
    fn retired_outcome(&self, id: ChildHandleId, acknowledged: bool) -> Option<TerminationOutcome> {
        let record = self.retired_record(id)?;
        let exit = record.exit.clone().unwrap_or_else(|| {
            ChildExit::Unobservable("retired before its exit was recorded".into())
        });
        let negative = record.fallback_authorised_by.clone().unwrap_or_default();
        Some(if record.sent.contains(&SentSignal::Kill) {
            TerminationOutcome::ExitedAfterKill { negative, exit }
        } else if record.sent.contains(&SentSignal::Term) {
            TerminationOutcome::ExitedAfterTerm { negative, exit }
        } else if acknowledged {
            TerminationOutcome::ExitedAfterProtocol(exit)
        } else {
            TerminationOutcome::AlreadyExited(exit)
        })
    }

    /// A handle that is neither held nor recorded: nothing to report but
    /// that nothing is retained.
    fn gone(&self, id: ChildHandleId, acknowledged: bool) -> TerminationOutcome {
        self.retired_outcome(id, acknowledged)
            .unwrap_or(TerminationOutcome::NoRetainedHandle)
    }

    fn retired_record(&self, id: ChildHandleId) -> Option<RetiredRecord> {
        self.retired
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|record| record.id == id)
            .cloned()
    }

    /// Number of live slots (unreaped or reaped-but-not-retired).
    pub fn slot_count(&self) -> usize {
        self.lock().len()
    }

    /// Whether an unreaped handle is retained for `id`.
    pub fn retains(&self, id: ChildHandleId) -> bool {
        self.lock().get(&id).is_some_and(|slot| !slot.reaped)
    }

    /// Whether `id` was ever adopted here (reaped or not).
    pub fn knows(&self, id: ChildHandleId) -> bool {
        self.lock().contains_key(&id)
    }

    /// Every adopted handle, for ownership assertions.
    pub fn handles(&self) -> Vec<ChildHandleId> {
        let mut ids: Vec<_> = self.lock().keys().copied().collect();
        ids.sort();
        ids
    }

    /// Signals actually dispatched for `id`, in order (at most one of each);
    /// a retired handle answers from its bounded record.
    pub fn signals_sent(&self, id: ChildHandleId) -> Vec<SentSignal> {
        if let Some(slot) = self.lock().get(&id) {
            return slot.sent.clone();
        }
        self.retired_record(id)
            .map(|record| record.sent)
            .unwrap_or_default()
    }

    /// The negative protocol outcome that authorised the fallback, if any
    /// step of it ran.
    pub fn fallback_authorised_by(&self, id: ChildHandleId) -> Option<String> {
        if let Some(slot) = self.lock().get(&id) {
            return slot.fallback_authorised_by.clone();
        }
        self.retired_record(id)
            .and_then(|record| record.fallback_authorised_by)
    }

    /// Exit observer: resolves `Some` once the reap task has reaped.
    pub fn exit_receiver(&self, id: ChildHandleId) -> Option<watch::Receiver<Option<ChildExit>>> {
        self.lock().get(&id).map(|slot| slot.exit.subscribe())
    }

    /// Wait for the child's exit; `None` for an unknown handle.
    pub async fn wait_exit(&self, id: ChildHandleId) -> Option<ChildExit> {
        let mut receiver = self.exit_receiver(id)?;
        loop {
            if let Some(exit) = receiver.borrow().clone() {
                return Some(exit);
            }
            if receiver.changed().await.is_err() {
                return receiver.borrow().clone();
            }
        }
    }

    async fn wait_exit_within(&self, id: ChildHandleId, budget: Duration) -> Option<ChildExit> {
        tokio::time::timeout(budget, self.wait_exit(id))
            .await
            .unwrap_or_default()
    }

    fn already_exited(&self, id: ChildHandleId) -> Option<ChildExit> {
        self.lock()
            .get(&id)
            .filter(|slot| slot.reaped)
            .and_then(|slot| slot.exit.borrow().clone())
    }

    /// Terminate an owned child: await the caller's `protocol` attempt
    /// first; on a negative outcome (or when an acknowledged child does not
    /// exit within `budget.exit_after_ack`) fall back to TERM, then KILL.
    /// Every branch signals at most once per kind and never after the reap.
    pub async fn terminate<P>(
        &self,
        id: ChildHandleId,
        protocol: P,
        budget: TerminationBudget,
    ) -> TerminationOutcome
    where
        P: Future<Output = ProtocolOutcome>,
    {
        if !self.knows(id) {
            return TerminationOutcome::NoRetainedHandle;
        }
        if let Some(exit) = self.already_exited(id) {
            return TerminationOutcome::AlreadyExited(exit);
        }
        // The protocol is always attempted first, whatever the caller's
        // urgency; the fallback is authorised only by its negative outcome.
        // A slot retired by its owner while this call was parked (the child
        // exited on its own and the reaper finished) is reported from its
        // record at every step below, never as a fabricated "still running".
        let negative = match protocol.await {
            ProtocolOutcome::Acknowledged => {
                match self.wait_exit_within(id, budget.exit_after_ack).await {
                    Some(exit) => return self.classify(id, None, exit),
                    None if !self.knows(id) => return self.gone(id, true),
                    None => format!(
                        "acknowledged but not exited within {:?}",
                        budget.exit_after_ack
                    ),
                }
            }
            ProtocolOutcome::Negative(detail) => detail,
        };
        if let Some(exit) = self.already_exited(id) {
            return TerminationOutcome::AlreadyExited(exit);
        }
        if !self.knows(id) {
            return self.gone(id, false);
        }
        self.dispatch(id, SentSignal::Term, &negative);
        match self.wait_exit_within(id, budget.term_grace).await {
            Some(exit) => return self.classify(id, Some(negative), exit),
            None if !self.knows(id) => return self.gone(id, false),
            None => {}
        }
        self.dispatch(id, SentSignal::Kill, &negative);
        match self.wait_exit_within(id, budget.kill_grace).await {
            Some(exit) => self.classify(id, Some(negative), exit),
            None if !self.knows(id) => self.gone(id, false),
            None => TerminationOutcome::StillRunning { negative },
        }
    }

    /// Name the outcome by what was actually sent to the handle — by this
    /// or any concurrent termination — so a caller parked in one phase never
    /// mislabels an exit another caller's later signal produced. `negative`
    /// is this caller's fallback authorisation; when it observed the exit
    /// during the protocol phase it carries the recorded one, if any.
    fn classify(
        &self,
        id: ChildHandleId,
        negative: Option<String>,
        exit: ChildExit,
    ) -> TerminationOutcome {
        let live = {
            let slots = self.lock();
            slots.get(&id).map(|slot| {
                (
                    slot.term_sent,
                    slot.kill_sent,
                    slot.fallback_authorised_by.clone(),
                )
            })
        };
        // Retired while this caller was parked: the record still says what
        // was sent, so the outcome is never downgraded.
        let (term_sent, kill_sent, recorded) = live
            .or_else(|| {
                self.retired_record(id).map(|record| {
                    (
                        record.sent.contains(&SentSignal::Term),
                        record.sent.contains(&SentSignal::Kill),
                        record.fallback_authorised_by,
                    )
                })
            })
            .unwrap_or((false, false, None));
        let negative = negative.or(recorded).unwrap_or_default();
        if kill_sent {
            TerminationOutcome::ExitedAfterKill { negative, exit }
        } else if term_sent {
            TerminationOutcome::ExitedAfterTerm { negative, exit }
        } else {
            TerminationOutcome::ExitedAfterProtocol(exit)
        }
    }

    /// Run [`Self::terminate`] detached on the supervisor's own runtime, for
    /// callers that are synchronous today (their orchestration migrates in
    /// #1936/#1938). Safe from any thread, inside or outside a runtime.
    pub fn request_termination(
        self: &Arc<Self>,
        id: ChildHandleId,
        protocol: std::pin::Pin<Box<dyn Future<Output = ProtocolOutcome> + Send>>,
        budget: TerminationBudget,
    ) {
        if !self.retains(id) {
            return;
        }
        let supervisor = Arc::clone(self);
        self.handle.spawn(async move {
            let outcome = supervisor.terminate(id, protocol, budget).await;
            tracing::info!(handle = ?id, ?outcome, "owned child termination finished");
        });
    }

    /// Send one signal kind at most once, only while the handle is unreaped,
    /// and record which negative protocol outcome authorised it.
    fn dispatch(&self, id: ChildHandleId, signal: SentSignal, negative: &str) {
        let mut slots = self.lock();
        let Some(slot) = slots.get_mut(&id) else {
            return;
        };
        if slot.reaped || slot.pid == 0 {
            return;
        }
        let already = match signal {
            SentSignal::Term => std::mem::replace(&mut slot.term_sent, true),
            SentSignal::Kill => std::mem::replace(&mut slot.kill_sent, true),
        };
        if already {
            return;
        }
        if slot.fallback_authorised_by.is_none() {
            slot.fallback_authorised_by = Some(negative.to_owned());
        }
        slot.sent.push(signal);
        #[cfg(any(test, feature = "test-support"))]
        if self.dry_run.load(Ordering::SeqCst) {
            return;
        }
        send_signal(slot.pid, slot.group, signal);
    }
}

/// Deliver `signal` to the retained child (or its own group). Called only
/// under the slot lock with `reaped == false`, so the pid is still the
/// child's. Zero and out-of-range pids are refused so nothing can address
/// the caller's own group.
fn send_signal(pid: u32, group: ProcessGroup, signal: SentSignal) {
    #[cfg(unix)]
    {
        let Ok(pid) = i32::try_from(pid) else {
            return;
        };
        if pid <= 0 {
            return;
        }
        let target = match group {
            ProcessGroup::Inherited => pid,
            ProcessGroup::Own => -pid,
        };
        let signal = match signal {
            SentSignal::Term => libc::SIGTERM,
            SentSignal::Kill => libc::SIGKILL,
        };
        // The pid is one this supervisor still retains unreaped (or the
        // process group it created for it) and the signal is a constant.
        // SAFETY: plain FFI call to `kill(2)` with validated arguments.
        unsafe {
            libc::kill(target, signal);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, group, signal);
    }
}

#[cfg(test)]
#[path = "owned_child_supervisor_tests.rs"]
mod tests;
