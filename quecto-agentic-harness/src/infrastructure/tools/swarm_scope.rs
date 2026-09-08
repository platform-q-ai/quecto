//! Own the interpreter until its ordinary execution group has been stopped.
use std::io;
use std::process::ExitStatus;
use std::sync::{Arc, Mutex};

use super::JobState;

pub(super) struct ExecutionScope {
    pub(super) child: tokio::process::Child,
    state: Option<Arc<Mutex<JobState>>>,
}

impl ExecutionScope {
    pub(super) fn new(child: tokio::process::Child, state: Option<Arc<Mutex<JobState>>>) -> Self {
        Self { child, state }
    }

    pub(super) fn terminate(&self) {
        if let Some(pid) = self.child.id() {
            // The owned child has not been reaped: its PID/group cannot be
            // recycled while we signal it, including on future cancellation.
            super::kill_pid_tree_best_effort(pid);
            super::kill_pid(pid);
        }
    }

    #[cfg(target_os = "linux")]
    pub(super) async fn wait(&mut self) -> io::Result<ExitStatus> {
        let pid = self.child.id().expect("execution scope still owns child");
        loop {
            if exited_without_reaping(pid)? {
                // Serialize reap + identity removal with registry cancellation.
                // Do not call Child::wait/try_wait before group cleanup: reaping
                // the leader first loses the reserved process-group identity.
                let mut state = self.state.as_ref().map(|s| s.lock().unwrap());
                super::kill_pid(pid);
                let result = self.child.try_wait();
                if let Some(state) = &mut state {
                    state.pid = None;
                }
                return result?
                    .ok_or_else(|| io::Error::other("exited interpreter was not waitable"));
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }

    // Production swarm admission requires Linux procfs. Keep the interpreter
    // helper buildable on other platforms for the rest of the harness.
    #[cfg(not(target_os = "linux"))]
    pub(super) async fn wait(&mut self) -> io::Result<ExitStatus> {
        let result = self.child.wait().await;
        if let Some(state) = &self.state {
            state.lock().unwrap().pid = None;
        }
        result
    }
}

impl Drop for ExecutionScope {
    fn drop(&mut self) {
        // On early return/drop, kill the scope before Tokio drops/reaps Child.
        // After a successful wait, Child::id is None and no PID is signalled.
        self.terminate();
        if let Some(state) = &self.state {
            state.lock().unwrap().pid = None;
        }
    }
}

#[cfg(target_os = "linux")]
fn exited_without_reaping(pid: u32) -> io::Result<bool> {
    // SAFETY: siginfo_t is a C output structure whose all-zero state is valid.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // WNOWAIT reserves the PID; WNOHANG keeps the executor thread nonblocking.
    // SAFETY: info is writable and pid identifies the owned child.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result != 0 {
        let error = io::Error::last_os_error();
        return if error.kind() == io::ErrorKind::Interrupted {
            Ok(false)
        } else {
            Err(error)
        };
    }
    // SAFETY: waitid initialized the SIGCHLD fields; zero means no child exit.
    Ok(unsafe { info.si_pid() } != 0)
}
