//! The test fixture binary's own contract (#2283 review): it fails loudly
//! rather than pass over what it cannot read or reach, as the Python
//! helpers it replaced did.
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const FIXTURE: &str = env!("CARGO_BIN_EXE_quecto-test-fixture");

fn fixture(args: &[&str]) -> Output {
    Command::new(FIXTURE)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("the fixture runs")
}

fn processes(dir: &std::path::Path, mode: &str, env: &str) -> Output {
    fixture(&["processes", mode, dir.to_str().unwrap(), env])
}

/// A record that exists but cannot be read is an error, and is kept: only
/// a record gone before it is read is skipped.
#[test]
fn a_record_that_cannot_be_read_fails_the_cleanup_and_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let record = dir.path().join("env.1.json");
    std::fs::write(&record, "not a record").unwrap();
    let cleaned = processes(dir.path(), "clean", "env");
    assert!(!cleaned.status.success(), "{cleaned:?}");
    assert!(record.exists(), "the unreadable record is kept");
    let live = processes(dir.path(), "live", "env");
    assert!(!live.status.success(), "{live:?}");
}

/// A pid no pidfd can be opened for, for any reason but its exit (here an
/// invalid pid, `EINVAL`), fails the cleanup: only `ESRCH` means gone.
#[test]
fn a_pid_that_cannot_be_opened_fails_the_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("env.1.json"), r#"[-5, "1"]"#).unwrap();
    let cleaned = processes(dir.path(), "clean", "env");
    assert!(!cleaned.status.success(), "{cleaned:?}");
    let reason = String::from_utf8_lossy(&cleaned.stderr);
    assert!(reason.contains("pidfd"), "{reason}");
}

/// A process that exited is gone: its record is dropped and the cleanup
/// passes.
#[test]
fn an_exited_process_is_cleaned_as_gone() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    std::fs::write(
        dir.path().join(format!("env.{pid}.json")),
        format!(r#"[{pid}, "0"]"#),
    )
    .unwrap();
    let cleaned = processes(dir.path(), "clean", "env");
    assert!(cleaned.status.success(), "{cleaned:?}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

/// The admission peer is bounded as the Python peer was: an endpoint that
/// accepts and never answers fails it within its 3 s bound, both direct
/// and through the proxy's nested peer.
#[test]
fn a_silent_endpoint_fails_the_admission_peer_within_its_bound() {
    let dir = tempfile::tempdir().unwrap();
    let endpoint = dir.path().join("silent.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&endpoint).unwrap();
    for mode in ["direct", "proxy"] {
        let started = Instant::now();
        let mut child = Command::new(FIXTURE)
            .args(["admission-peer", mode])
            .env("ADMISSION_ENDPOINT", &endpoint)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let request = br#"{"id":"x","version":1}"#;
        let mut frame = (request.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(request);
        std::io::Write::write_all(child.stdin.as_mut().unwrap(), &frame).unwrap();
        let exited = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if started.elapsed() > Duration::from_secs(10) {
                let _ = child.kill();
                panic!("{mode}: the peer outlived its bound");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(!exited.success(), "{mode}");
        assert!(started.elapsed() < Duration::from_secs(8), "{mode}");
    }
}
