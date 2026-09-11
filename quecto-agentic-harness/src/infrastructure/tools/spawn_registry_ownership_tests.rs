//! #1925: registry teardown may only signal pids this harness launched.
//!
//! BDD fixtures used to register entries with literal pids (1, 2, 42); inside
//! a swarm container pid 2 is the coordinator, and `shutdown_all` SIGTERMed
//! it. These tests pin the guard from both sides with REAL signals: an
//! unowned entry carrying our own pid must not reach us, and an owned entry
//! must still be terminated.

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

/// Install a counting SIGTERM handler for the lifetime of the probe.
struct SigtermProbe {
    previous: libc::sighandler_t,
}

impl SigtermProbe {
    fn install() -> Self {
        let handler = record_sigterm as extern "C" fn(libc::c_int) as libc::sighandler_t;
        // The previous disposition is restored on drop.
        // SAFETY: installs an async-signal-safe handler (an atomic add) for SIGTERM.
        let previous = unsafe { libc::signal(libc::SIGTERM, handler) };
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
        // SAFETY: restores the disposition captured by `install`.
        unsafe {
            libc::signal(libc::SIGTERM, self.previous);
        }
    }
}

// Serialises tests that install the process-wide SIGTERM handler.
static PROBE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    // Fixture-style entries: constructed, never launched by this process.
    let direct = SubagentEntry::new("/tmp/self-direct.sock".into(), own_pid);
    let mut group = SubagentEntry::new("/tmp/self-group.sock".into(), own_pid);
    group.process_owner = ProcessOwner::LocalProcessGroup;
    assert!(!direct.process_ownership.is_owned());
    assert!(!group.process_ownership.is_owned());

    // Cascade teardown of a removed entry.
    terminate_removed_entry(&direct);
    terminate_removed_entry(&group);
    assert_eq!(probe.deliveries(), 0, "cascade signalled an unowned pid");

    // Full registry shutdown.
    let registry = new_registry();
    {
        let mut guard = registry.lock().unwrap();
        guard.insert("direct".into(), direct);
        guard.insert("group".into(), group);
    }
    shutdown_all(&registry);
    assert!(registry.lock().unwrap().is_empty());
    assert_eq!(
        probe.deliveries(),
        0,
        "shutdown_all signalled an unowned pid"
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
}
