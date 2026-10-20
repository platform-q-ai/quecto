//! The multi-process admission tests' guard on their children (#2631): every
//! child is killed and reaped when its `Proc` is dropped (a failed
//! assertion), and dies with the thread that spawned it (a killed test
//! binary). Both are proven here, through the file's own `quecto()`.
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use quecto::infrastructure::processes::parent_death_signal::{DeathSignal, arm_in_child};

/// A child the test kills and reaps when it is dropped, so a failed
/// assertion never strands it (#2631). `into_inner` hands it over, unkilled,
/// to be waited on to its own end.
pub(super) struct Proc(Option<Child>);

impl Proc {
    pub(super) fn new(child: Child) -> Self {
        Self(Some(child))
    }

    pub(super) fn child(&mut self) -> &mut Child {
        self.0
            .as_mut()
            .expect("the child is held until it is handed over")
    }

    pub(super) fn id(&self) -> u32 {
        self.0
            .as_ref()
            .expect("the child is held until it is handed over")
            .id()
    }

    pub(super) fn into_inner(mut self) -> Child {
        self.0
            .take()
            .expect("the child is held until it is handed over")
    }
}

/// How long a dropped `Proc` waits for its killed child to be reaped: a
/// bound, so a child that somehow outlives its kill fails the test that
/// checks it rather than hanging the run (the parent-death signal ends it
/// when the test thread does).
const REAP_BOUND: Duration = Duration::from_secs(5);

impl Drop for Proc {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let deadline = Instant::now() + REAP_BOUND;
            while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

/// Every child of this test dies with the thread that spawned it, even when
/// the test binary itself is killed and no `Drop` runs (#2631): the kernel
/// sends it SIGTERM (Linux `PR_SET_PDEATHSIG`, armed as the harness arms its
/// own children).
pub(super) fn dies_with_the_test(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    let parent = libc::pid_t::try_from(std::process::id()).expect("a process id fits pid_t");
    // The closure runs in the forked child after fork and before exec, the
    // one place arm_in_child may run: it calls only async-signal-safe
    // functions (prctl, getppid, _exit), allocates nothing and touches no
    // memory of the parent; `parent` is a plain integer copied in.
    // SAFETY: that is exactly arm_in_child's contract.
    unsafe {
        command.pre_exec(move || arm_in_child(parent, DeathSignal::Terminate));
    }
}

/// A child spawned through the test's `quecto()` on a thread that then ends
/// is ended by the kernel, with nothing reaping it: a UDS root, which
/// otherwise runs until it is killed.
#[cfg(target_os = "linux")]
#[test]
fn a_quecto_child_dies_with_the_thread_that_spawned_it() {
    let temp = tempfile::tempdir().unwrap();
    let config = super::write_config(temp.path(), "http://127.0.0.1:9", None);
    let socket = temp.path().join("run").join("orphan.sock");
    let child = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let (config, socket) = (config.to_str().unwrap(), socket.to_str().unwrap());
                let args = [
                    "--config",
                    config,
                    "agent",
                    "--mode",
                    "uds",
                    "--no-session",
                    "--socket",
                    socket,
                ];
                super::quecto(temp.path(), &args)
                    .spawn()
                    .expect("spawn uds root")
            })
            .join()
            .expect("the spawning thread ends")
    });
    let mut child = Proc::new(child);
    let deadline = Instant::now() + super::LIMIT;
    let status = loop {
        if let Some(status) = child.child().try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "the orphaned UDS root kept running"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    use std::os::unix::process::ExitStatusExt;
    assert_eq!(
        status.signal(),
        Some(libc::SIGTERM),
        "the root ended by the parent-death signal, not on its own: {status:?}"
    );
}

/// Dropping a `Proc` kills and reaps its child at once: no process is left
/// at its pid, and the drop does not wait out the child.
#[test]
fn dropping_a_proc_leaves_no_process() {
    let mut command = Command::new("sleep");
    command.arg("30");
    dies_with_the_test(&mut command);
    let child = Proc::new(command.spawn().expect("spawn sleep"));
    let pid = libc::pid_t::try_from(child.id()).expect("a process id fits pid_t");
    let started = Instant::now();
    drop(child);
    assert!(
        started.elapsed() < REAP_BOUND,
        "the drop waited out the child"
    );
    // SAFETY: signal 0 only checks that the pid exists; nothing is sent.
    let alive = unsafe { libc::kill(pid, 0) };
    assert_eq!(alive, -1, "no process is left at {pid}");
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}
