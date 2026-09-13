//! Bounded-concurrency settlement of independent futures (#1938).
//!
//! The fleet teardown settles every direct child through the same
//! claim → edge → conclude → compensate sequence; children are independent,
//! so they settle concurrently, but never more than `limit` at once, so a
//! large fleet cannot open an unbounded number of edges or fallback
//! budgets simultaneously. Pure `std`: no runtime, no task spawning; the
//! caller's executor polls this one future and it polls the active set.
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

pub(super) type Settlement<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// Resolves once every queued settlement has produced its output, keeping
/// at most `limit` in flight. Outputs are collected in completion order;
/// callers that need a stable order sort afterwards.
pub(super) struct BoundedSettlement<T> {
    queued: VecDeque<Settlement<T>>,
    active: Vec<Settlement<T>>,
    done: Vec<T>,
    limit: usize,
}

impl<T> BoundedSettlement<T> {
    pub(super) fn new(limit: usize, queued: Vec<Settlement<T>>) -> Self {
        assert!(
            limit >= 1,
            "a settlement bound of zero could never progress"
        );
        Self {
            queued: queued.into(),
            active: Vec::with_capacity(limit),
            done: Vec::new(),
            limit,
        }
    }

    fn refill(&mut self) {
        while self.active.len() < self.limit {
            let Some(next) = self.queued.pop_front() else {
                break;
            };
            self.active.push(next);
        }
    }
}

impl<T> Unpin for BoundedSettlement<T> {}

impl<T> Future for BoundedSettlement<T> {
    type Output = Vec<T>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        loop {
            this.refill();
            if this.active.is_empty() {
                debug_assert!(this.queued.is_empty(), "refill drains the queue first");
                return Poll::Ready(std::mem::take(&mut this.done));
            }
            let mut progressed = false;
            let mut index = 0;
            while index < this.active.len() {
                match this.active[index].as_mut().poll(cx) {
                    Poll::Ready(output) => {
                        this.done.push(output);
                        // Swap-remove keeps every other active future polled
                        // in this pass; completion order is not promised.
                        drop(this.active.swap_remove(index));
                        progressed = true;
                    }
                    Poll::Pending => index += 1,
                }
            }
            if !progressed {
                return Poll::Pending;
            }
        }
    }
}

#[cfg(test)]
#[path = "bounded_settlement_tests.rs"]
mod tests;
