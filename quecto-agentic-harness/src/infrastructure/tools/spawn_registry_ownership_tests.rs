//! #1925: registry teardown may only signal pids this harness launched or
//! that were reported from its own pid namespace.
//!
//! BDD fixtures used to register entries with literal pids (1, 2, 42); inside
//! a swarm container pid 2 is the coordinator, and `shutdown_all` SIGTERMed
//! it. These tests pin the guard from both sides with REAL signals: an
//! unowned entry carrying our own pid must not reach us, and an owned entry
//! must still be terminated.
//!
//! Constraint: the self-pid probe installs a process-wide SIGTERM disposition
//! for the whole test binary. It is serialised through `PROBE_LOCK`, saved and
//! restored with `sigaction`, and no other test in this binary may rely on the
//! default SIGTERM disposition while it runs.

use super::super::process_ownership::ProcessOwnership;
use super::super::process_tree::ProcessOwner;
use super::super::subagent_cascade::terminate_removed_entry;
use super::super::subagent_registry::{SubagentEntry, new_registry};
use super::shutdown_all;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

static SIGTERM_DELIVERIES: AtomicUsize = AtomicUsize::new(0);

extern "C" fn record_sigterm(_signal: libc::c_int) {
    SIGTERM_DELIVERIES.fetch_add(1, Ordering::SeqCst);
}

/// Install a counting SIGTERM handler for the lifetime of the probe and
/// restore the exact previous `sigaction` on drop.
struct SigtermProbe {
    previous: libc::sigaction,
}

impl SigtermProbe {
    fn install() -> Self {
        // SAFETY: zeroed sigaction is a valid all-default action to fill in.
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = record_sigterm as extern "C" fn(libc::c_int) as libc::sighandler_t;
        // SAFETY: empties the handler's blocked-signal mask before install.
        unsafe { libc::sigemptyset(&mut action.sa_mask) };
        action.sa_flags = libc::SA_RESTART;
        // SAFETY: zeroed sigaction is a valid out-parameter for the old action.
        let mut previous: libc::sigaction = unsafe { std::mem::zeroed() };
        // SAFETY: installs an async-signal-safe handler (an atomic add) and captures the old one.
        let installed = unsafe { libc::sigaction(libc::SIGTERM, &action, &mut previous) };
        assert_eq!(installed, 0, "sigaction(SIGTERM) must succeed");
        SIGTERM_DELIVERIES.store(0, Ordering::SeqCst);
        Self { previous }
    }

    fn deliveries(&self) -> usize {
        // A self-directed signal is delivered before `kill` returns to the
        // calling thread, but give the kernel a beat in case it picked another.
        std::thread::sleep(Duration::from_millis(50));
        SIGTERM_DELIVERIES.load(Ordering::SeqCst)
    }
}

impl Drop for SigtermProbe {
    fn drop(&mut self) {
        // SAFETY: restores the exact disposition captured by `install`.
        unsafe {
            libc::sigaction(libc::SIGTERM, &self.previous, std::ptr::null_mut());
        }
    }
}

// Serialises tests that install the process-wide SIGTERM handler.
static PROBE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn spawn_sleep_in_own_group() -> tokio::process::Child {
    let mut command = tokio::process::Command::new("sleep");
    command
        .arg("30")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // A fresh process group with pgid == pid, exactly like a local launch, so
    // a LocalProcessGroup lease targets ONLY this helper, never our group.
    command.process_group(0);
    command.spawn().expect("spawn sleep")
}

#[test]
fn unowned_entry_carrying_our_own_pid_is_never_signalled() {
    let _serial = PROBE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let probe = SigtermProbe::install();
    let own_pid = std::process::id();

    // Positive control: the probe really observes a SIGTERM aimed at us.
    // SAFETY: self-directed SIGTERM with the counting handler installed.
    unsafe {
        libc::kill(own_pid as libc::pid_t, libc::SIGTERM);
    }
    assert_eq!(probe.deliveries(), 1, "probe must observe a real SIGTERM");
    SIGTERM_DELIVERIES.store(0, Ordering::SeqCst);

    // Fixture-style entry: constructed, never launched by this process. Only
    // the DirectPid topology is probed against our own pid: a group lease on
    // our pid would be a group signal, which is covered by the helper below.
    let direct = SubagentEntry::new("/tmp/self-direct.sock".into(), own_pid);
    assert!(!direct.process_ownership.is_owned());

    // Cascade teardown of a removed entry.
    terminate_removed_entry(&direct);
    assert_eq!(probe.deliveries(), 0, "cascade signalled an unowned pid");

    // Full registry shutdown.
    let registry = new_registry();
    registry.lock().unwrap().insert("direct".into(), direct);
    shutdown_all(&registry);
    assert!(registry.lock().unwrap().is_empty());
    assert_eq!(
        probe.deliveries(),
        0,
        "shutdown_all signalled an unowned pid"
    );
}

#[tokio::test]
async fn unowned_group_entry_leaves_a_foreign_process_group_alone() {
    let mut helper = spawn_sleep_in_own_group();
    let pid = helper.id().expect("live helper pid");
    let mut group = SubagentEntry::new("/tmp/foreign-group.sock".into(), pid);
    group.process_owner = ProcessOwner::LocalProcessGroup;
    assert!(!group.process_ownership.is_owned());

    terminate_removed_entry(&group);
    let registry = new_registry();
    registry.lock().unwrap().insert("group".into(), group);
    shutdown_all(&registry);

    tokio::time::sleep(Duration::from_millis(150)).await;
    let still_running = matches!(helper.try_wait(), Ok(None));
    let _ = helper.kill().await;
    let _ = helper.wait().await;
    assert!(
        still_running,
        "an unowned group lease must not signal a foreign process group"
    );
}

#[tokio::test]
async fn owned_launched_child_is_still_terminated_on_shutdown() {
    let mut child = tokio::process::Command::new("sleep")
        .arg("30")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn sleep");
    let pid = child.id().expect("live child pid");

    let mut entry = SubagentEntry::new("/tmp/owned-child.sock".into(), pid);
    entry.process_ownership = ProcessOwnership::launched(&child);
    assert!(entry.process_ownership.is_owned());
    let registry = new_registry();
    registry.lock().unwrap().insert("owned".into(), entry);

    shutdown_all(&registry);

    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("owned child must exit after shutdown_all")
        .expect("wait");
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            status.signal(),
            Some(libc::SIGTERM),
            "child must die by SIGTERM"
        );
    }
}

#[tokio::test]
async fn owned_group_lease_terminates_the_launched_process_group() {
    let mut child = spawn_sleep_in_own_group();
    let pid = child.id().expect("live child pid");
    let mut entry = SubagentEntry::new("/tmp/owned-group.sock".into(), pid);
    entry.process_owner = ProcessOwner::LocalProcessGroup;
    entry.process_ownership = ProcessOwnership::launched(&child);
    let registry = new_registry();
    registry.lock().unwrap().insert("owned".into(), entry);

    shutdown_all(&registry);

    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("owned group must exit after shutdown_all")
        .expect("wait");
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert!(
            matches!(status.signal(), Some(libc::SIGTERM | libc::SIGKILL)),
            "group leader must die by TERM or the KILL follow-up: {status:?}"
        );
    }
}

#[test]
fn entry_constructors_default_to_unowned() {
    let by_new = SubagentEntry::new("/tmp/a.sock".into(), 0);
    assert!(!by_new.process_ownership.is_owned());
    let by_identity = SubagentEntry::with_identity(
        crate::domain::ids::AgentUuid::mint(),
        "b".into(),
        "/tmp/b.sock".into(),
        0,
    );
    assert!(!by_identity.process_ownership.is_owned());
    // A clone shares the lease; it cannot manufacture authority.
    assert!(!by_identity.clone().process_ownership.is_owned());
    // A foreign-namespace report grants nothing; a same-namespace one does.
    assert!(!ProcessOwnership::reported(false).is_owned());
    assert!(ProcessOwnership::reported(true).is_owned());
}
