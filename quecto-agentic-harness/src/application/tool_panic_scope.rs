//! The tool-execution panic scope (#2192, ADR-0029).
//!
//! A panic is contained only where the harness chose to contain it: inside a
//! tool call. Everywhere else it stays fail-fast — the process-wide panic
//! hook aborts. The hook runs synchronously on the panicking thread, so it
//! can only learn where it is from that thread; this module keeps that answer
//! in a thread-local that is set exactly while a tool call's code runs.
//!
//! The marker travels with the call's future, not with a thread: [`scoped`]
//! wraps the future, and every `poll` enters the scope for its own duration
//! (restoring whatever was there before, even while unwinding). A call that
//! awaits, moves to another worker thread and is polled again is therefore
//! in scope on every thread it runs on, and never outside it. The scope
//! closes when the call's future is dropped: after that nothing is in it.
//!
//! Work a call hands to another task or thread does not inherit the scope by
//! itself — a long-lived background task a tool starts (a monitor, a reaper)
//! is not the call's work, and a panic there must stay fatal. Work that IS
//! the call's own opts in through [`carry`] (a blocking closure) or
//! [`carry_future`] (a spawned future), and is joined so that its panic
//! resumes in the call (`infrastructure::tools::call_work`); once the call
//! has ended, carried work that is still running is outside any scope.
//!
//! Code a call runs that handles a panic itself catches it through
//! [`catch_in_call`], which ends the contained unwind and forgets the
//! handled panic; a raw `catch_unwind` would leave both behind.
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, TryLockError};
use std::task::{Context, Poll};

/// Where and why a tool's code panicked, as the hook saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanicSite {
    pub message: String,
    /// `file:line:column`, when the panic reported one.
    pub location: Option<String>,
}

/// A panic recorded on a scope, and the thread that raised it: a panic the
/// same thread then catches itself ([`catch_in_call`]) is forgotten.
#[derive(Debug)]
struct Recorded {
    site: PanicSite,
    thread: u64,
}

/// The panics recorded on one scope: the first one kept (the call's cause),
/// and the first panic of each other thread after it, so that forgetting
/// the kept one — its thread caught it itself — leaves the scope failed by
/// another thread's (#2192 review: a detached thread's panic is never lost
/// to the recording thread's catch).
#[derive(Debug, Default)]
struct Records {
    kept: Option<Recorded>,
    /// At most [`MAX_LATER_THREADS`] entries, one per thread. When a further
    /// thread panics, the last entry is pinned (its thread set to the token
    /// 0, under which nothing is forgotten) instead: the scope then stays
    /// failed whatever is caught later.
    later: Vec<Recorded>,
}

/// How many threads besides the kept panic's have their panic remembered.
const MAX_LATER_THREADS: usize = 8;

impl Records {
    fn has_thread(&self, thread: u64) -> bool {
        self.kept
            .iter()
            .chain(&self.later)
            .any(|recorded| recorded.thread == thread)
    }

    /// Record `recorded`, answering whether it became the kept panic.
    fn record(&mut self, recorded: Recorded) -> bool {
        match (&self.kept, self.has_thread(recorded.thread)) {
            (None, _) => {
                self.kept = Some(recorded);
                true
            }
            // A thread's later panic is a consequence of its first.
            (Some(_), true) => false,
            (Some(_), false) if self.later.len() < MAX_LATER_THREADS => {
                self.later.push(recorded);
                false
            }
            (Some(_), false) => {
                if let Some(last) = self.later.last_mut() {
                    last.thread = PINNED_THREAD;
                }
                false
            }
        }
    }

    /// Remove `thread`'s panics; the kept one, when it was `thread`'s, is
    /// answered, and the earliest other thread's becomes the kept one. A
    /// thread that is not live (token 0) forgets nothing.
    fn forget(&mut self, thread: u64) -> Option<Recorded> {
        match thread {
            PINNED_THREAD => None,
            live => self.forget_live(live),
        }
    }

    fn forget_live(&mut self, thread: u64) -> Option<Recorded> {
        assert_ne!(thread, PINNED_THREAD, "only a live thread forgets");
        self.later.retain(|recorded| recorded.thread != thread);
        match &self.kept {
            Some(kept) if kept.thread == thread => {
                let forgotten = self.kept.take();
                self.kept = match self.later.is_empty() {
                    true => None,
                    false => Some(self.later.remove(0)),
                };
                forgotten
            }
            Some(_) | None => None,
        }
    }
}

/// The thread token of a recorded panic nothing can forget: a thread whose
/// thread-locals are torn down, or an entry pinned by overflow.
const PINNED_THREAD: u64 = 0;

/// What recording a panic on a scope did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recording {
    /// It is the call's panic: the first one.
    Kept,
    /// It was remembered after the first (another thread's), or dropped as
    /// a consequence of one already recorded.
    Remembered,
    /// The scope is closed (or its record could not be had): nothing was
    /// recorded, and the panic is not the call's to contain.
    Refused,
}

/// One tool call's scope: which tool, the panics seen inside it, and
/// whether the call is still running. The panics are kept behind a mutex
/// that is held only to set, read or forget them — never across a panic —
/// and the hook takes it with bounded `try_lock` retries, so it never waits.
#[derive(Debug)]
pub struct ToolScope {
    tool: String,
    site: Mutex<Records>,
    open: AtomicBool,
}

impl ToolScope {
    pub fn new(tool: impl Into<String>) -> Arc<Self> {
        let tool = tool.into();
        assert!(!tool.is_empty(), "a tool scope names its tool");
        Arc::new(Self {
            tool,
            site: Mutex::new(Records::default()),
            open: AtomicBool::new(true),
        })
    }

    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// Whether the call is still running: a closed scope contains nothing.
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    /// Close the scope: nothing is recorded on it once this returns. The
    /// record lock is taken after the flag is cleared, so a record already
    /// in progress finishes first (and a reader after the close sees it),
    /// and any later one sees the scope closed and is refused (#2192
    /// review). Not hook-safe (it waits); only a call's end closes.
    fn close(&self) {
        self.open.store(false, Ordering::Release);
        drop(
            self.site
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
    }

    /// Record a panic the calling thread raised in this scope. The first one
    /// is kept: it is the cause, and anything after it on the same thread is
    /// a consequence. Another thread's first panic is remembered too, so the
    /// scope stays failed if the kept one is forgotten. A closed scope
    /// records nothing: its call's result is already read, so the panic is
    /// not the call's to contain. Hook-safe: it never blocks and never
    /// panics.
    pub fn record_panic(&self, site: PanicSite) -> Recording {
        let thread = thread_token();
        self.with_site(|records| match self.open.load(Ordering::Acquire) {
            true => match records.record(Recorded { site, thread }) {
                true => Recording::Kept,
                false => Recording::Remembered,
            },
            false => Recording::Refused,
        })
        .unwrap_or(Recording::Refused)
    }

    /// The recorded panic, if any. Hook-safe.
    pub fn recorded_panic(&self) -> Option<PanicSite> {
        self.with_site(|records| records.kept.as_ref().map(|recorded| recorded.site.clone()))
            .flatten()
    }

    /// Whether the calling thread has a panic recorded here.
    fn has_own_panic(&self) -> bool {
        let thread = thread_token();
        self.with_site(|records| records.has_thread(thread))
            .unwrap_or(false)
    }

    /// Forget the panics the calling thread raised: that thread caught them
    /// itself, so they are not the call's. Another thread's stay.
    fn forget_own_panic(&self) {
        let thread = thread_token();
        let forgotten = self.with_site(|records| records.forget(thread));
        // Dropped outside the lock: freeing never runs under it.
        drop(forgotten);
    }

    /// Run `f` on the recorded panics, retrying a contended lock a bounded
    /// number of times; `None` when it could not be had. Hook-safe.
    fn with_site<R>(&self, f: impl FnOnce(&mut Records) -> R) -> Option<R> {
        for attempt in 0..SITE_ATTEMPTS {
            match self.site.try_lock() {
                Ok(mut slot) => return Some(f(&mut slot)),
                Err(TryLockError::Poisoned(poisoned)) => {
                    return Some(f(&mut poisoned.into_inner()));
                }
                Err(TryLockError::WouldBlock) if attempt % 64 == 63 => std::thread::yield_now(),
                Err(TryLockError::WouldBlock) => std::hint::spin_loop(),
            }
        }
        None
    }
}

/// How many times a scope's panic record is tried before giving up: it is
/// held only to set, read or forget one record.
const SITE_ATTEMPTS: usize = 100_000;

static NEXT_THREAD_TOKEN: AtomicU64 = AtomicU64::new(1);

/// This thread's token among those that recorded panics: never 0 for a
/// live thread; 0 ([`PINNED_THREAD`]) while its thread-locals are torn
/// down, a token no recorded panic can be forgotten under. Hook-safe.
fn thread_token() -> u64 {
    THREAD_TOKEN
        .try_with(|token| match token.get() {
            0 => {
                let fresh = NEXT_THREAD_TOKEN.fetch_add(1, Ordering::Relaxed);
                token.set(fresh);
                fresh
            }
            known => known,
        })
        .unwrap_or(0)
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<ToolScope>>> = const { RefCell::new(None) };
    /// A contained panic is unwinding on this thread: set by the hook when it
    /// lets a panic unwind, cleared once that unwind is over — by the
    /// containment that caught it, or when a scope is next entered or left
    /// with no panic in progress. Never while unwinding: a scope a
    /// destructor enters and leaves during the unwind does not end it.
    static CONTAINED_UNWIND: Cell<bool> = const { Cell::new(false) };
    /// This thread's [`thread_token`], 0 until first asked for.
    static THREAD_TOKEN: Cell<u64> = const { Cell::new(0) };
}

/// The open tool scope the calling thread is running in, if any. Safe to
/// call from a panic hook: it never panics, and a thread-local that is being
/// torn down or is mid-update reads as "no scope" (the fail-fast answer).
pub fn current() -> Option<Arc<ToolScope>> {
    CURRENT
        .try_with(|slot| slot.try_borrow().ok().and_then(|scope| scope.clone()))
        .ok()
        .flatten()
        .filter(|scope| scope.is_open())
}

/// Mark that a contained panic is unwinding on this thread, answering
/// whether one already was: a second panic before the first left its scope
/// (a destructor that panics during the unwind) cannot be contained — Rust
/// aborts on it — so the hook treats it as fatal. Hook-safe.
pub fn begin_contained_unwind() -> bool {
    CONTAINED_UNWIND
        .try_with(|unwinding| unwinding.replace(true))
        .unwrap_or(true)
}

/// The contained unwind on this thread is over: its panic was caught.
pub fn end_contained_unwind() {
    let _ = CONTAINED_UNWIND.try_with(|unwinding| unwinding.set(false));
}

/// Clear a contained unwind that is over: none is while no panic is in
/// progress on this thread.
fn settle_contained_unwind() {
    match std::thread::panicking() {
        true => {}
        false => end_contained_unwind(),
    }
}

/// The scope entered on this thread until dropped; the drop restores what
/// was there before, including while a panic unwinds through it.
struct Entered {
    previous: Option<Arc<ToolScope>>,
    entered: bool,
}

fn enter(scope: Option<Arc<ToolScope>>) -> Entered {
    settle_contained_unwind();
    let swapped = CURRENT.try_with(|slot| {
        slot.try_borrow_mut()
            .map(|mut slot| std::mem::replace(&mut *slot, scope))
            .ok()
    });
    match swapped {
        Ok(Some(previous)) => Entered {
            previous,
            entered: true,
        },
        // No slot to set: the code runs unmarked, so a panic in it is fatal.
        Ok(None) | Err(_) => Entered {
            previous: None,
            entered: false,
        },
    }
}

impl Drop for Entered {
    fn drop(&mut self) {
        settle_contained_unwind();
        if self.entered {
            let previous = self.previous.take();
            let _ = CURRENT.try_with(|slot| {
                if let Ok(mut slot) = slot.try_borrow_mut() {
                    *slot = previous;
                }
            });
        }
    }
}

/// Run `f` inside `scope` on the calling thread.
pub fn run_in<T>(scope: &Arc<ToolScope>, f: impl FnOnce() -> T) -> T {
    let _entered = enter(Some(scope.clone()));
    f()
}

/// The tool calls running now, by call id: what a fatal panic names as the
/// calls that were running when it happened.
static IN_FLIGHT: Mutex<Vec<(u64, String)>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// How many times the hook tries the in-flight registry before giving up:
/// it is held only for a push or a removal, never across a panic.
const IN_FLIGHT_ATTEMPTS: usize = 100_000;

fn register_in_flight(tool: &str) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let entry = (id, tool.to_string());
    IN_FLIGHT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(entry);
    id
}

fn deregister_in_flight(id: u64) {
    let removed = {
        let mut calls = IN_FLIGHT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        calls
            .iter()
            .position(|(call, _)| *call == id)
            .map(|at| calls.remove(at))
    };
    // Dropped outside the lock: freeing never runs under it.
    drop(removed);
}

/// The tools of the calls running now, oldest first. Hook-safe: it never
/// blocks, retrying a contended registry a bounded number of times.
pub fn in_flight_tools() -> Vec<String> {
    for attempt in 0..IN_FLIGHT_ATTEMPTS {
        match IN_FLIGHT.try_lock() {
            Ok(calls) => return calls.iter().map(|(_, tool)| tool.clone()).collect(),
            Err(TryLockError::Poisoned(poisoned)) => {
                return poisoned
                    .into_inner()
                    .iter()
                    .map(|(_, tool)| tool.clone())
                    .collect();
            }
            Err(TryLockError::WouldBlock) if attempt % 64 == 63 => std::thread::yield_now(),
            Err(TryLockError::WouldBlock) => std::hint::spin_loop(),
        }
    }
    Vec::new()
}

/// Run `f`, catching a panic in it — the one way code a tool call runs
/// handles a panic itself (#2192; architecture test `panics_are_caught_in_the_sanctioned_way`).
/// A panic caught here is over: the unwind the hook let through ends, and
/// the panic it recorded on the call's scope is forgotten, so a later panic
/// is contained as the call's first and a call that handled its panic still
/// succeeds. A panic this thread recorded before `f` ran, or one another
/// thread recorded, is kept. Outside a scope it only catches.
///
/// Known limit (#2192 review): it cannot catch a panic raised while a
/// contained panic is already unwinding on this thread — code a destructor
/// runs during that unwind. The hook sees a second panic in the scope and
/// ends the process before the unwind reaches this catch, exactly as it
/// would for an uncaught second panic.
pub fn catch_in_call<T>(f: impl FnOnce() -> T + std::panic::UnwindSafe) -> std::thread::Result<T> {
    let scope = current();
    let recorded_before = scope.as_deref().is_some_and(ToolScope::has_own_panic);
    let caught = std::panic::catch_unwind(f);
    match (&caught, &scope, recorded_before) {
        (Err(_), Some(scope), false) => {
            settle_contained_unwind();
            scope.forget_own_panic();
        }
        (Err(_), _, _) => settle_contained_unwind(),
        (Ok(_), _, _) => {}
    }
    caught
}

/// A future whose every poll runs inside a scope.
pub struct InScope<F> {
    scope: Option<Arc<ToolScope>>,
    inner: Pin<Box<F>>,
    /// The call's id among the calls in flight: set for the call's own
    /// future, whose drop ends the call; never for carried work.
    call: Option<u64>,
}

impl<F> Drop for InScope<F> {
    fn drop(&mut self) {
        if let Some(id) = self.call.take() {
            deregister_in_flight(id);
            if let Some(scope) = &self.scope {
                scope.close();
            }
        }
    }
}

impl<F: Future> Future for InScope<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        let this = self.get_mut();
        let _entered = enter(this.scope.clone());
        this.inner.as_mut().poll(cx)
    }
}

/// A tool call's `future`, polled inside `scope` wherever and however often
/// it is polled; dropping it ends the call and closes the scope.
pub fn scoped<F: Future>(scope: Arc<ToolScope>, future: F) -> InScope<F> {
    assert!(scope.is_open(), "a call runs in a fresh scope");
    InScope {
        call: Some(register_in_flight(scope.tool())),
        scope: Some(scope),
        inner: Box::pin(future),
    }
}

/// A blocking job that runs in the caller's tool scope, for the call's own
/// work handed to another thread (`spawn_blocking`). Outside a scope it runs
/// unmarked, exactly as the bare job would.
pub fn carry<T>(job: impl FnOnce() -> T) -> impl FnOnce() -> T {
    let scope = current();
    move || match scope {
        Some(scope) => run_in(&scope, job),
        None => job(),
    }
}

/// A future that runs in the caller's tool scope, for the call's own work
/// handed to a spawned task. Outside a scope it runs unmarked.
pub fn carry_future<F: Future>(future: F) -> InScope<F> {
    InScope {
        scope: current(),
        inner: Box::pin(future),
        call: None,
    }
}

/// The `source` of the `error` event a contained tool panic records.
pub const TOOL_PANIC_SOURCE: &str = "tool_panic";

/// The text of a panic's payload: `panic!` gives a `&'static str` for a
/// literal message and a `String` for a formatted one; anything else (a
/// `panic_any` value) has no text to show.
pub fn payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&'static str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "a panic with no message".to_string()
    }
}

#[cfg(test)]
#[path = "tool_panic_scope_tests.rs"]
mod tests;
