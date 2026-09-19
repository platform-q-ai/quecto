//! A retained inspect script can block for its full timeout. UDS readiness must
//! not wait for that blocking startup reconciliation.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const READY_BOUND: Duration = Duration::from_secs(3);

fn wait_for(path: &Path, child: &mut Child, what: &str) {
    let deadline = Instant::now() + READY_BOUND;
    while !path.exists() {
        if let Some(status) = child.try_wait().expect("query harness") {
            panic!("harness exited before {what}: {status}");
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn uds_serves_get_state_while_a_restored_environment_inspection_hangs() {
    let temp = tempfile::tempdir().expect("temporary base");
    let base = temp.path();
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let config = base.join("config.json");
    std::fs::write(
        &config,
        format!(
            r#"{{"providers":{{"openai":{{"api_key":"sk-test","api_base":"http://127.0.0.1:1"}}}},"agents":{{"defaults":{{"model":"openai-api/gpt-4o-mini","workspace":{}}}}}}}"#,
            serde_json::to_string(&workspace).unwrap()
        ),
    )
    .unwrap();

    let marker = base.join("inspect-started");
    let inspect = base.join("inspect.sh");
    std::fs::write(
        &inspect,
        format!("#!/bin/sh\ntouch '{}'\nsleep 30\n", marker.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&inspect, std::fs::Permissions::from_mode(0o755)).unwrap();

    let registry = serde_json::json!({
        "version": 1,
        "next_ref": 1,
        "environments": {
            "C1": {
                "environment_id": "env-hanging",
                "environment_uuid": "uuid-hanging",
                "workspace_path": workspace,
                "repository": "",
                "config": "test",
                "exec": [],
                "kill": [],
                "cleanup": [],
                "inspect": [inspect],
                "status": "running",
                "metadata": {},
                "created_by": "cli:earlier",
                "created_at": 1
            }
        }
    });
    std::fs::write(
        base.join("environments.json"),
        serde_json::to_vec_pretty(&registry).unwrap(),
    )
    .unwrap();

    let socket = base.join("agent.sock");
    let mut child = Command::new(env!("CARGO_BIN_EXE_quecto"))
        .args(["agent", "--mode", "uds", "--persist", "--socket"])
        .arg(&socket)
        .args(["--config"])
        .arg(&config)
        .env("QUECTO_BASE_DIR", base)
        .env("HOME", base)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn UDS harness");

    wait_for(&marker, &mut child, "retained inspect to start");
    assert!(
        socket.exists(),
        "the socket must be bound before reconciliation starts"
    );

    let mut stream = UnixStream::connect(&socket).expect("connect while inspect hangs");
    stream.set_read_timeout(Some(READY_BOUND)).unwrap();
    stream
        .write_all(b"{\"type\":\"get_state\",\"id\":\"startup-ready\"}\n")
        .unwrap();
    let mut reader = BufReader::new(stream);
    let response = loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .expect("read get_state response");
        assert!(!line.is_empty(), "socket closed before get_state response");
        let event: serde_json::Value = serde_json::from_str(&line).unwrap();
        if event["id"] == "startup-ready" {
            break event;
        }
    };
    assert_eq!(response["success"], true, "{response}");

    child.kill().ok();
    child.wait().ok();
}
