use std::{sync::mpsc, time::Duration};

pub const A: &str = "0123456789abcdef";
pub const B: &str = "fedcba9876543210";

/// A hard per-probe bound independent of SQLite/Git retry implementations.
/// Normal unwind cancels and joins it; a hung probe terminates without a core dump.
pub struct GitRemoteScalarWatchdog(
    Option<mpsc::Sender<()>>,
    Option<std::thread::JoinHandle<()>>,
);
impl GitRemoteScalarWatchdog {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            if matches!(
                rx.recv_timeout(Duration::from_secs(45)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                // SAFETY: _exit takes only a scalar status, never returns, and has no memory preconditions.
                unsafe { libc::_exit(124) }
            }
        });
        Self(Some(tx), Some(thread))
    }
}
impl Drop for GitRemoteScalarWatchdog {
    fn drop(&mut self) {
        self.0.take().unwrap().send(()).ok();
        self.1.take().unwrap().join().unwrap();
    }
}
