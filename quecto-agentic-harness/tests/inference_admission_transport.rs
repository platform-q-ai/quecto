//! P0 transport experiment only: real local scripts, not Docker, admission policy,
//! authentication, or production create/join wiring. The host uses shared framing.
#![cfg(unix)]

use std::process::Stdio;
use std::time::{Duration, Instant};

use quecto_line_io::{FrameError, read_frame, write_frame};
use serde_json::{Value, json};
use tokio::io::BufReader;
use tokio::net::UnixListener;
use tokio::process::{Child, Command};
use tokio::time::timeout;

const CAP: usize = 4096;
const DEADLINE: Duration = Duration::from_secs(5);
const SCRIPT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/inference_admission/peer.py"
);

// Also kills nested bridge descendants if an assertion/read panics. The direct
// child is reaped within a bounded interval; Python normally reaps its own child.
struct ProcessGuard(Child, u32);
impl Drop for ProcessGuard {
    fn drop(&mut self) {
        // SAFETY: each fixture starts a fresh process group whose id we own.
        unsafe { libc::kill(-(self.1 as i32), libc::SIGKILL) };
        let until = Instant::now() + Duration::from_secs(1);
        while Instant::now() < until {
            if !matches!(self.0.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = self.0.start_kill(); // kill_on_drop remains a final fallback.
    }
}

fn launch(mode: &str, endpoint: &std::path::Path) -> ProcessGuard {
    let mut command = Command::new("/usr/bin/python3");
    command
        .args(["-I", SCRIPT, mode])
        .env_clear()
        .env("LC_ALL", "C")
        .env("ADMISSION_ENDPOINT", endpoint)
        .env("ADMISSION_SCOPE", "fixture-scope")
        .env("ADMISSION_CAPABILITY", "fixture-only-not-a-provider-secret")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command.process_group(0);
    let child = command.spawn().expect("python3 fixture available");
    let pid = child.id().unwrap();
    ProcessGuard(child, pid)
}

async fn exchange(child: &mut ProcessGuard, request: &Value) -> Value {
    write_frame(
        child.0.stdin.as_mut().unwrap(),
        &serde_json::to_vec(request).unwrap(),
        CAP,
    )
    .await
    .unwrap();
    let bytes = read_frame(&mut BufReader::new(child.0.stdout.as_mut().unwrap()), CAP)
        .await
        .unwrap()
        .unwrap();
    let reply = serde_json::from_slice(&bytes).unwrap();
    assert!(child.0.wait().await.unwrap().success(), "child_exit");
    reply
}

// No detached task: cancellation of the enclosing deadline drops accept/read
// futures and sockets. The observations come from this actual listener, never
// from the client report or a canned authority echo.
async fn observe(listener: &UnixListener, authority: &str, serial: usize) -> (Value, Value) {
    let (stream, _) = listener.accept().await.unwrap();
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read);
    let (observed, status) = match read_frame(&mut read, CAP).await {
        Err(FrameError::Oversized { declared, max }) => {
            (json!({"declared": declared, "max": max}), "oversized")
        }
        Ok(Some(bytes)) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(request) => {
                let status = if request["version"] != 1 {
                    "unsupported-version"
                } else if request["capability"] != "reverse-stdio-v1" {
                    "unsupported-capability"
                } else {
                    "observed"
                };
                (request, status)
            }
            Err(_) => (json!({"bytes": bytes}), "malformed"),
        },
        other => panic!("unexpected framing result: {other:?}"),
    };
    let reply = json!({"authority": authority, "serial": serial, "epoch": 7,
                      "id": observed.get("id"), "scope": observed.get("scope"), "status": status});
    write_frame(&mut write, &serde_json::to_vec(&reply).unwrap(), CAP)
        .await
        .unwrap();
    (observed, reply)
}

#[tokio::test]
async fn direct_proxy_nested_and_rejections_reach_one_private_listener() {
    let dir = tempfile::tempdir().unwrap();
    let endpoint = dir.path().join("admission.sock");
    let listener = UnixListener::bind(&endpoint).unwrap();
    let authority = uuid::Uuid::new_v4().to_string();
    let cases = [
        ("direct", "direct", 1, "reverse-stdio-v1", "observed"),
        ("proxy", "nested", 1, "reverse-stdio-v1", "observed"),
        (
            "direct",
            "version",
            99,
            "reverse-stdio-v1",
            "unsupported-version",
        ),
        (
            "proxy",
            "capability",
            1,
            "unknown",
            "unsupported-capability",
        ),
        ("direct", "malformed", 1, "reverse-stdio-v1", "malformed"),
        ("proxy", "oversized", 1, "reverse-stdio-v1", "oversized"),
    ];
    let mut pids = Vec::new();
    for (serial, (mode, id, version, capability, status)) in cases.into_iter().enumerate() {
        let request = json!({"id": id, "version": version, "capability": capability,
                             "epoch": 7, "scope": "fixture-scope"});
        let mut child = launch(mode, &endpoint);
        let launcher_pid = child.1;
        let ((observed, sent), received) = timeout(DEADLINE, async {
            tokio::join!(
                observe(&listener, &authority, serial),
                exchange(&mut child, &request)
            )
        })
        .await
        .expect("bounded transport deadline");
        assert_eq!(received, sent, "correlated_reply");
        assert_eq!(received["status"], status, "rejection_status");
        assert_eq!(received["authority"], authority, "same_authority");
        assert_eq!(received["serial"], serial, "broker_serial");
        assert_eq!(received["epoch"], 7, "reply_epoch");
        if id == "malformed" {
            assert_eq!(observed, json!({"bytes": b"{broken"}), "malformed_bytes");
        } else if id == "oversized" {
            assert_eq!(
                observed,
                json!({"declared": CAP + 1, "max": CAP}),
                "oversized_bound"
            );
        } else {
            let mut envelope = observed.clone();
            let peer = envelope.as_object_mut().unwrap().remove("peer").unwrap();
            assert_eq!(envelope, request, "exact_broker_request");
            assert_eq!(received["id"], id, "reply_id");
            assert_eq!(received["scope"], "fixture-scope", "reply_scope");
            let expected_mode = if mode == "proxy" { "nested" } else { "direct" };
            assert_eq!(peer["argv"], json!([expected_mode]), "restricted_argv");
            assert_eq!(
                peer["env"],
                json!({"LC_ALL": "C",
                "ADMISSION_ENDPOINT": endpoint, "ADMISSION_SCOPE": "fixture-scope",
                "ADMISSION_CAPABILITY": "fixture-only-not-a-provider-secret"}),
                "restricted_env"
            );
            let pid = peer["pid"].as_u64().unwrap() as u32;
            assert_ne!(pid, std::process::id(), "separate_process");
            assert!(!pids.contains(&pid), "distinct_peers");
            pids.push(pid);
            if mode == "proxy" {
                assert_ne!(pid, launcher_pid, "nested_pid");
                assert_eq!(peer["ppid"], launcher_pid, "nested_parent");
            } else {
                assert_eq!(pid, launcher_pid, "direct_pid");
            }
        }
    }
    assert!(
        timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err(),
        "no_extra_requests"
    );
}

#[tokio::test]
async fn missing_endpoint_fails_closed_in_direct_and_proxy() {
    let dir = tempfile::tempdir().unwrap();
    for mode in ["direct", "proxy"] {
        let mut child = launch(mode, &dir.path().join("missing.sock"));
        timeout(DEADLINE, async {
            write_frame(child.0.stdin.as_mut().unwrap(), br#"{"id":"missing"}"#, CAP)
                .await
                .unwrap();
            let response = read_frame(&mut BufReader::new(child.0.stdout.as_mut().unwrap()), CAP)
                .await
                .unwrap();
            assert!(response.is_none(), "missing_no_response");
            assert!(!child.0.wait().await.unwrap().success(), "missing_exit");
        })
        .await
        .expect("bounded missing endpoint failure");
    }
}
