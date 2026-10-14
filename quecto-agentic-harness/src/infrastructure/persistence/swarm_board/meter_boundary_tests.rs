//! Bounded FFI failure and unobserved allocation regressions.
use super::{MeteredConnection, Tally};
use rusqlite::Connection;
use std::thread;
use std::time::{Duration, Instant};

/// Arbitrary panic payload destructors must never unwind across SQLite's C ABI.
#[test]
fn panicking_payload_drop_is_contained_in_bounded_child() {
    const CHILD: &str = "QUECTO_METER_PANIC_PAYLOAD_CHILD";
    if std::env::var(CHILD).as_deref() == Ok("1") {
        struct Bomb;
        impl Drop for Bomb {
            fn drop(&mut self) {
                panic!("panic payload destructor");
            }
        }
        let mut tally = Tally::new();
        tally.observe(std::sync::Arc::new(|_| std::panic::panic_any(Bomb)));
        let connection =
            MeteredConnection::new(Connection::open_in_memory().unwrap(), Some(&tally)).unwrap();
        let handler = connection.handler.as_ref().unwrap();
        let argument = std::ptr::from_ref(&**handler).cast_mut().cast();
        // SAFETY: owned handler remains alive and registered throughout this synchronous call.
        let retry = unsafe { super::busy_callback(argument, 0) };
        assert_eq!(retry, 0, "panic payload safely refused");
        assert!(tally.held().busy_wait >= Duration::from_millis(1));
        return;
    }
    let scratch = tempfile::TempDir::new().unwrap();
    let output = std::fs::File::create(scratch.path().join("child-output")).unwrap();
    struct ProbeChild(std::process::Child);
    impl Drop for ProbeChild {
        fn drop(&mut self) {
            // Every error/unwind path also hard-kills and reaps the child.
            let _killed = self.0.kill();
            let _reaped = self.0.wait();
        }
    }
    let mut child = ProbeChild(std::process::Command::new("bash")
        .args(["-c", "ulimit -c 0 && ulimit -f 128 && exec \"$@\"", "meter-child"])
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", "infrastructure::persistence::swarm_board::meter::boundary_tests::panicking_payload_drop_is_contained_in_bounded_child", "--nocapture"])
        .env(CHILD, "1")
        .stdout(output.try_clone().unwrap())
        .stderr(output)
        .spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break Some(status);
        }
        if Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        } else {
            child.0.kill().expect("SIGKILL overdue child");
            child.0.wait().expect("reap killed child");
            break None;
        }
    };
    let diagnostic = std::fs::read_to_string(scratch.path().join("child-output")).unwrap();
    scratch
        .close()
        .expect("remove child output including on failure");
    assert!(
        status.is_some_and(|status| status.success()),
        "child must safely refuse: {status:?}: {diagnostic}"
    );
}

#[test]
fn unobserved_connection_has_no_observation_phase_or_identity_work() {
    let tally = Tally::new();
    assert!(
        tally.observation.is_none(),
        "no observation UUID or sequence"
    );
    let connection =
        MeteredConnection::new(Connection::open_in_memory().unwrap(), Some(&tally)).unwrap();
    assert!(
        connection.phase_control().is_none(),
        "no observer means no phase Arc/state"
    );
    assert!(
        connection.handler.as_ref().unwrap().observation.is_none(),
        "no receipt mutex or scope"
    );
    let handler = connection.handler.as_ref().unwrap();
    handler.account(0, Duration::from_millis(105));
    assert_eq!(
        tally.held().busy_wait,
        Duration::from_millis(105),
        "direct tally accounting retained"
    );
    connection.close().unwrap();
}
