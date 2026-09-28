//! A tool call's own work, handed to another task or thread (#2192,
//! ADR-0029). The work runs in the call's panic scope, and joining it
//! resumes a panic it raised in the call itself — where the agent loop's
//! containment turns it into that call's internal error. Swallowing the
//! panic instead (reading a `JoinError` as "no output") would answer the
//! model with a wrong result that looks like success.
//!
//! This is the only place infrastructure carries a call's scope: every
//! carried tokio task is spawned here and can only be joined through
//! [`CarriedJoin`] (architecture test `carried_work_is_joined`). The one
//! exception is [`std_thread_in_call`]: its std thread is detached (the
//! `find` owner drops the handle), so its panic never resumes in the call.
//! It is still the call's: recorded on the call's scope, it makes the agent
//! loop's backstop (`execute_contained`) fail a call that then returns
//! normally.
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::application::tool_panic_scope::{carry, carry_future};

/// A carried task's handle. Awaiting it yields the task's value, or the
/// `JoinError` of a task that was cancelled; a task that panicked resumes
/// its panic in the awaiting call.
#[derive(Debug)]
pub struct CarriedJoin<T>(tokio::task::JoinHandle<T>);

/// Join `handle`, resuming its panic in the caller: the one way a call reads
/// work it carried.
pub fn join_carried<T>(handle: tokio::task::JoinHandle<T>) -> CarriedJoin<T> {
    CarriedJoin(handle)
}

impl<T> CarriedJoin<T> {
    /// Cancel the task (a helper the call no longer needs).
    pub fn abort_handle(&self) -> tokio::task::AbortHandle {
        self.0.abort_handle()
    }
}

impl<T> Future for CarriedJoin<T> {
    type Output = Result<T, tokio::task::JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match Pin::new(&mut self.0).poll(cx) {
            Poll::Ready(Err(error)) if error.is_panic() => {
                std::panic::resume_unwind(error.into_panic())
            }
            other => other,
        }
    }
}

/// Run `job` on the blocking pool as the calling tool call's own work.
pub fn spawn_blocking_in_call<T, F>(job: F) -> CarriedJoin<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    join_carried(tokio::task::spawn_blocking(carry(job)))
}

/// As [`spawn_blocking_in_call`], on a given runtime's blocking pool.
pub fn spawn_blocking_in_call_on<T, F>(runtime: &tokio::runtime::Handle, job: F) -> CarriedJoin<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    join_carried(runtime.spawn_blocking(carry(job)))
}

/// Start a thread whose job is the calling tool call's own work: it runs in
/// the call's scope, so a panic in it while the call runs is the call's
/// (recorded on its scope; the call fails with it through
/// `execute_contained`'s backstop, since a detached thread's panic never
/// resumes in the call) rather than the process's. A thread that outlives
/// its call is outside the scope then.
pub fn std_thread_in_call<T, F>(
    builder: std::thread::Builder,
    job: F,
) -> std::io::Result<std::thread::JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    builder.spawn(carry(job))
}

/// Run `future` as a task that is the calling tool call's own work.
pub fn spawn_in_call<F>(future: F) -> CarriedJoin<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    join_carried(tokio::spawn(carry_future(future)))
}

#[cfg(test)]
#[path = "call_work_tests.rs"]
mod tests;
