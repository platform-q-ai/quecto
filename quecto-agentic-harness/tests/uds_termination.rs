//! A UDS harness must exit cleanly on SIGTERM instead of dying on the
//! signal's default action. The clean path is what tears down container
//! environments (see `src/interface/cli/uds_shutdown.rs`); a process killed
//! by the signal itself has run none of it.
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn quecto_binary_path() -> PathBuf {
    if let Some(p) = option_env!("CARGO_BIN_EXE_quecto") {
        return PathBuf::from(p);
    }
    let test_exe = std::env::current_exe().expect("get current exe");
    let debug_dir = test_exe
        .parent()
        .and_then(|p| p.parent())
        .expect("debug dir");
    debug_dir.join("quecto")
}

#[test]
fn uds_harness_exits_cleanly_on_sigterm() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("agent.sock");
    // A configured provider is a startup precondition; no request is made.
    std::fs::write(
        dir.path().join("config.json"),
        r#"{"providers":{"anthropic":{"api_key":"test-key-never-used"}}}"#,
    )
    .unwrap();
    let mut child = std::process::Command::new(quecto_binary_path())
        .args(["agent", "--mode", "uds", "--socket"])
        .arg(&socket)
        .env("QUECTO_BASE_DIR", dir.path())
        .env("HOME", dir.path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn uds harness");

    let deadline = Instant::now() + Duration::from_secs(20);
    while !socket.exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("harness exited before binding its socket: {status}");
        }
        assert!(
            Instant::now() < deadline,
            "harness never bound {}",
            socket.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    let status = std::process::Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());

    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("harness did not exit within 10s of SIGTERM");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(
        status.signal(),
        None,
        "harness must handle SIGTERM (run subagent/environment teardown) rather than die on it: {status}"
    );
    assert_eq!(status.code(), Some(0), "{status}");
}
