//! A shared cancellation flag that can be both checked and watched (#2155).
//!
//! Setting it wakes every watcher, so a wait for cancellation costs nothing
//! until the cancel: no timer, no polling loop. The domain stays pure and
//! synchronous: a [`CancelWatch`] registers a `std` [`Waker`], and the outer
//! layers turn it into whatever wait their runtime needs.
//!
//! No wake-up is ever lost. A watch checks the flag, then under the waiter
//! lock checks it again and registers its waker; a cancel sets the flag
//! before it takes the lock and wakes every registered waker. Whichever takes
//! the lock first, the other sees it: the watch sees the flag set, or the
//! cancel sees the waker.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::task::Waker;

/// A shared cancellation flag that providers can check and watch.
///
/// A domain-level concept, so the domain layer does not expose raw
/// concurrency primitives in its public API. Clones share one flag.
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<Shared>);

#[derive(Debug, Default)]
struct Shared {
    cancelled: AtomicBool,
    waiters: Mutex<Waiters>,
}

/// The wakers of the watches pending on the flag, each under its watch's id.
#[derive(Debug, Default)]
struct Waiters {
    next: u64,
    wakers: Vec<(u64, Waker)>,
}

impl CancelFlag {
    /// Create a new, unset cancel flag.
    pub fn new() -> Self {
        Self::default()
    }

    /// Signal cancellation: every watch of the flag, from any clone, is woken.
    /// The next provider check will return a cancellation error.
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        // The flag is set before the lock is taken: a watch registering
        // after this drain sees it set under the same lock.
        let woken = std::mem::take(&mut self.0.lock().wakers);
        for (_, waker) in woken {
            waker.wake();
        }
    }

    /// Returns `true` if cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }

    /// A watch of the flag, which registers a waker for its cancel; see
    /// [`CancelWatch::cancelled_or_wake`].
    pub fn watch(&self) -> CancelWatch {
        CancelWatch {
            flag: self.clone(),
            slot: None,
        }
    }

    /// How many watches are registered on the flag.
    #[cfg(test)]
    fn waiters(&self) -> usize {
        self.0.lock().wakers.len()
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Waiters> {
        // The waiter list stays consistent through any panic: each
        // operation on it is a single push, update or removal.
        self.waiters.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// One watcher of a [`CancelFlag`]: at most one waker registered at a time,
/// deregistered when the watch is dropped or sees the cancel.
#[derive(Debug)]
pub struct CancelWatch {
    flag: CancelFlag,
    /// The id this watch's waker is registered under, once registered.
    slot: Option<u64>,
}

impl CancelWatch {
    /// `true` once the flag is cancelled. Until then, `waker` is registered
    /// (replacing this watch's earlier one) and the cancel wakes it, once.
    pub fn cancelled_or_wake(&mut self, waker: &Waker) -> bool {
        if self.flag.is_cancelled() {
            self.deregister();
            return true;
        }
        let mut waiters = self.flag.0.lock();
        // Checked again under the lock: a cancel that set the flag before
        // this lock was taken has drained, or will never see, this waker.
        if self.flag.is_cancelled() {
            drop(waiters);
            self.deregister();
            return true;
        }
        let registered = self
            .slot
            .and_then(|id| waiters.wakers.iter_mut().find(|(slot, _)| *slot == id));
        match registered {
            Some((_, registered)) => registered.clone_from(waker),
            None => {
                let id = waiters.next;
                waiters.next = waiters
                    .next
                    .checked_add(1)
                    .expect("watch ids never run out");
                waiters.wakers.push((id, waker.clone()));
                self.slot = Some(id);
            }
        }
        debug_assert!(
            waiters
                .wakers
                .iter()
                .filter(|(slot, _)| Some(*slot) == self.slot)
                .count()
                == 1,
            "a pending watch is registered exactly once"
        );
        false
    }

    fn deregister(&mut self) {
        if let Some(id) = self.slot.take() {
            self.flag.0.lock().wakers.retain(|(slot, _)| *slot != id);
        }
    }
}

impl Drop for CancelWatch {
    fn drop(&mut self) {
        self.deregister();
    }
}

#[cfg(test)]
#[path = "cancel_flag_tests.rs"]
mod tests;
