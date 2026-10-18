//! Adapter-local immutable observations; never an application timing port.
#[cfg(any(test, feature = "test-support"))]
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    #[cfg(any(test, feature = "test-support"))]
    Setup,
    Begin,
    #[cfg(any(test, feature = "test-support"))]
    Work,
    #[cfg(any(test, feature = "test-support"))]
    Commit,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    pub call: u128,
    pub connection: u64,
    pub transaction: u64,
    pub phase: Phase,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug)]
pub struct AccountedSample {
    pub scope: Scope,
    pub callback: i32,
    pub actual: Duration,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy, Debug)]
pub enum Observation {
    Begin(Scope),
    Accounted(AccountedSample),
}

/// The same affirmative release policy drives fake cases and both real fixtures.
/// No clock, planned delay, or setup time is a qualification input.
#[cfg(any(test, feature = "test-support"))]
pub fn release_qualified(held_ack: bool, intended: Scope, sample: AccountedSample) -> bool {
    held_ack
        && intended.phase == Phase::Begin
        && sample.scope == intended
        && sample.callback >= 0
        && sample.actual >= Duration::from_millis(100)
}

/// Shared fixture lifecycle: the decision is invoked even before BEGIN or a
/// receipt exists. Absence is an input, never a caller-side short circuit.
#[cfg(any(test, feature = "test-support"))]
#[derive(Debug)]
pub struct ReleaseLifecycle {
    held_ack: bool,
    intended: Option<Scope>,
    receipt: Option<AccountedSample>,
}

#[cfg(any(test, feature = "test-support"))]
impl ReleaseLifecycle {
    pub fn new(held_ack: bool) -> Self {
        Self {
            held_ack,
            intended: None,
            receipt: None,
        }
    }

    pub fn observe(&mut self, event: Observation) -> bool {
        match event {
            Observation::Begin(scope) if self.intended.is_none() => self.intended = Some(scope),
            Observation::Accounted(sample) => self.receipt = Some(sample),
            _ => {}
        }
        self.qualified()
    }

    pub fn qualified(&self) -> bool {
        match (self.intended, self.receipt) {
            (Some(intended), Some(receipt)) => release_qualified(self.held_ack, intended, receipt),
            _ => false,
        }
    }
}

/// Test adapter fixture with joined, bounded cancellation cleanup. Dropping it
/// releases the holder on error/unwind; no detached releaser or contender exists.
#[cfg(any(test, feature = "test-support"))]
pub struct HeldLock {
    release: std::sync::Arc<std::sync::Mutex<Option<std::sync::mpsc::Sender<bool>>>>,
    thread: Option<std::thread::JoinHandle<rusqlite::Result<bool>>>,
}

#[cfg(any(test, feature = "test-support"))]
impl HeldLock {
    pub fn take(path: std::path::PathBuf) -> Self {
        let (held, taken) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            let connection = rusqlite::Connection::open(path)?;
            connection.busy_timeout(Duration::from_millis(500))?;
            connection.execute_batch("BEGIN IMMEDIATE")?;
            let qualified = held.send(()).is_ok()
                && released
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap_or(false);
            connection.execute_batch(if qualified { "COMMIT" } else { "ROLLBACK" })?;
            Ok(qualified)
        });
        let holder = Self {
            release: std::sync::Arc::new(std::sync::Mutex::new(Some(release))),
            thread: Some(thread),
        };
        taken
            .recv_timeout(Duration::from_secs(2))
            .expect("holder acknowledged lock");
        holder
    }

    pub fn observer(&self) -> std::sync::Arc<dyn Fn(Observation) + Send + Sync> {
        let release = self.release.clone();
        let lifecycle = std::sync::Mutex::new(ReleaseLifecycle::new(true));
        std::sync::Arc::new(move |event| {
            let qualifies = lifecycle
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .observe(event);
            if qualifies {
                if let Some(release) = release
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .take()
                {
                    let _sent = release.send(true);
                }
            }
        })
    }

    pub fn finish(mut self) {
        self.cancel();
        assert!(
            self.thread
                .take()
                .expect("holder thread")
                .join()
                .expect("holder panicked")
                .expect("holder failed"),
            "holder cancelled or timed out without a qualified release"
        );
    }

    fn cancel(&self) {
        if let Some(release) = self
            .release
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _sent = release.send(false);
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for HeldLock {
    fn drop(&mut self) {
        self.cancel();
        if let Some(thread) = self.thread.take() {
            let _joined = thread.join();
        }
    }
}

#[cfg(test)]
#[path = "meter_observation_tests.rs"]
mod tests;
