//! Adapter-local immutable observations; never an application timing port.
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Setup,
    Begin,
    Work,
    Commit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope {
    pub call: u128,
    pub connection: u64,
    pub transaction: u64,
    pub phase: Phase,
}

#[derive(Clone, Copy, Debug)]
pub struct AccountedSample {
    pub scope: Scope,
    pub callback: i32,
    pub actual: Duration,
}

#[derive(Clone, Copy, Debug)]
pub enum Observation {
    Begin(Scope),
    Accounted(AccountedSample),
}

/// The same affirmative release policy drives fake cases and both real fixtures.
/// No clock, planned delay, or setup time is a qualification input.
pub fn release_qualified(held_ack: bool, intended: Scope, sample: AccountedSample) -> bool {
    held_ack
        && intended.phase == Phase::Begin
        && sample.scope == intended
        && sample.callback >= 0
        && sample.actual >= Duration::from_millis(100)
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
        let intended = std::sync::Mutex::new(None);
        std::sync::Arc::new(move |event| {
            let mut intended = intended
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match event {
                Observation::Begin(scope) if intended.is_none() => *intended = Some(scope),
                Observation::Accounted(sample) => {
                    if let Some(scope) = *intended {
                        if release_qualified(true, scope, sample) {
                            if let Some(release) = release
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .take()
                            {
                                let _sent = release.send(true);
                            }
                        }
                    }
                }
                _ => {}
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
