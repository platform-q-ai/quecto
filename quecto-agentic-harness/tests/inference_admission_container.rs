//! P3 real container evidence: a Docker/Podman child launched through the
//! bundled adapter binds its admission capability through the identity-mounted
//! client directory before announcing socket readiness, waits behind its root
//! and is granted only when the root releases capacity.
//!
//! Requires a container runtime and the `quecto-box:local` image, so it runs
//! only when `QUECTO_ADMISSION_CONTAINER_E2E=1`; otherwise it reports the
//! reason and passes without claiming container evidence.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use quecto::domain::inference_admission::{Feedback, GroupId, WorkloadClass};
use quecto::infrastructure::admission::{
    AdminConnection, AuthorityConnection, AuthorityDirectory, write_admission_context,
};

const LIMIT: Duration = Duration::from_secs(90);

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn runtime_cli() -> Option<&'static str> {
    for cli in ["podman", "docker"] {
        let ok = Command::new(cli)
            .args(["image", "inspect", "quecto-box:local"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Some(cli);
        }
    }
    None
}

struct Proc(Child);
impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_connectable(socket: &Path, what: &str, mut alive: impl FnMut() -> bool) {
    let deadline = Instant::now() + LIMIT;
    while std::os::unix::net::UnixStream::connect(socket).is_err() {
        assert!(
            alive(),
            "{what} exited before {} was ready",
            socket.display()
        );
        assert!(
            Instant::now() < deadline,
            "{what} never accepted on {}",
            socket.display()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn container_child_negotiates_admission_through_the_mounted_client_directory() {
    if std::env::var("QUECTO_ADMISSION_CONTAINER_E2E").as_deref() != Ok("1") {
        eprintln!(
            "skipped: set QUECTO_ADMISSION_CONTAINER_E2E=1 with podman/docker and quecto-box:local"
        );
        return;
    }
    let cli = runtime_cli().expect("podman or docker with quecto-box:local");
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let base = home.join(".quecto");
    let run = home.join("run");
    let state = home.join("state");
    let authority = home.join("authority");
    for dir in [&base, &run, &state, &home.join("workspace")] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let config = base.join("config.json");
    std::fs::write(
        &config,
        format!(
            r#"{{"agents":{{"defaults":{{"workspace":{ws:?},"model":"fake/test-model"}}}},
"providers":{{"openai_compatible":{{"endpoints":[{{"prefix":"fake","api_key":"k","api_base":"http://127.0.0.1:9","allow_remote_http":true}}]}}}},
"admission":{{"directory":{auth:?},"groups":{{"g":{{"capacity":1,"reserve":0,"min_interval_ms":1,"queue_capacity":8,"queue_timeout_ms":60000,"attempt_timeout_ms":120000,"fallback_base_ms":100,"max_cooldown_ms":10000}}}},"aliases":{{"acct":"g"}},"bindings":{{"fake":"acct"}}}}}}"#,
            ws = home.join("workspace").to_string_lossy(),
            auth = authority.to_string_lossy()
        ),
    )
    .unwrap();
    let quecto = PathBuf::from(env!("CARGO_BIN_EXE_quecto"));
    let mut broker = Command::new(&quecto)
        .args([
            "--config",
            config.to_str().unwrap(),
            "admission-broker",
            "run",
        ])
        .env("HOME", home)
        .env("QUECTO_BASE_DIR", &base)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let dir = AuthorityDirectory::open(&authority).unwrap();
    wait_connectable(&dir.client_socket(), "authority", || {
        broker.try_wait().unwrap().is_none()
    });
    let _broker = Proc(broker);

    // The test is the root: it holds the only slot and pre-registers the child.
    let root = AuthorityConnection::connect(&dir.client_socket())
        .await
        .unwrap();
    let credential = root
        .register_root(WorkloadClass::Interactive)
        .await
        .unwrap();
    root.bind(credential).await.unwrap();
    let child = root.register_child().await.unwrap();
    let context = run.join("quecto-admission-child.json");
    write_admission_context(&context, &dir.client_socket(), &child).unwrap();
    let permit = root.gate("acct").unwrap().acquire().await.unwrap();

    // Launch exactly as spawn_container does: create.sh -- <binary> <child args>.
    let create = repo_root().join("scripts/container-runtime/docker/create.sh");
    let child_socket = run.join("quecto-agent-child.sock");
    let output = Command::new(&create)
        .args(["--state-dir", state.to_str().unwrap(), "--"])
        .arg(&quecto)
        .args([
            "agent",
            "--mode",
            "uds",
            "-s",
            "child",
            "--socket",
            child_socket.to_str().unwrap(),
            "--persist",
            "--spawned",
            "--config",
            config.to_str().unwrap(),
            "--admission-context",
            context.to_str().unwrap(),
        ])
        .env("HOME", home)
        .env("QUECTO_BASE_DIR", &base)
        .env("QUECTO_CONTAINER_CLI", cli)
        .env("QUECTO_CONTAINER_CONFIG", "docker")
        .env("QUECTO_CONTAINER_ENVIRONMENT_REF", "ref-admission-e2e")
        .env("QUECTO_ADMISSION_DIR", dir.client_dir())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "create failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        result["admission_capability"], "shared-directory-v1",
        "adapter reports the mounted capability: {result}"
    );
    let environment_id = result["environment_id"].as_str().unwrap().to_owned();
    let kill = repo_root().join("scripts/container-runtime/docker/kill.sh");
    let teardown = || {
        let _ = Command::new(&kill)
            .args(["--state-dir", state.to_str().unwrap()])
            .env("QUECTO_CONTAINER_CLI", cli)
            .env("QUECTO_CONTAINER_ENVIRONMENT_ID", &environment_id)
            .status();
    };

    // Socket readiness implies the child bound its capability through the mount.
    wait_connectable(&child_socket, "container child", || true);
    let admin = AdminConnection::connect(&dir.admin_socket()).await.unwrap();
    let group = GroupId::new("g").unwrap();
    let prompt = serde_json::json!({"type":"prompt","message":"hello","ack":"accept"}).to_string();
    quecto::infrastructure::tools::subagent_registry::send_subagent_uds_command(
        &child_socket,
        &prompt,
    )
    .await
    .unwrap();
    let deadline = Instant::now() + LIMIT;
    loop {
        let status = admin.inspect().await.unwrap();
        let g = status.groups[&group];
        if g.queued == 1 && g.active == 1 {
            break;
        }
        if Instant::now() >= deadline {
            teardown();
            panic!("container child never queued behind the root: {g:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    permit.finish(Feedback::Success);
    let deadline = Instant::now() + LIMIT;
    loop {
        let status = admin.inspect().await.unwrap();
        let g = status.groups[&group];
        if g.queued == 0 {
            assert_eq!(g.uncertain, 0, "grant is a verified transition: {g:?}");
            break;
        }
        if Instant::now() >= deadline {
            teardown();
            panic!("container child never granted after release: {g:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    teardown();

    // Without the directory the adapter reports no capability, so an enabled
    // parent would refuse the launch before inference.
    let output = Command::new(&create)
        .args(["--state-dir", state.to_str().unwrap(), "--"])
        .arg(&quecto)
        .args([
            "agent",
            "--mode",
            "uds",
            "--socket",
            run.join("plain.sock").to_str().unwrap(),
        ])
        .env("HOME", home)
        .env("QUECTO_BASE_DIR", &base)
        .env("QUECTO_CONTAINER_CLI", cli)
        .env("QUECTO_CONTAINER_CONFIG", "docker")
        .env("QUECTO_CONTAINER_ENVIRONMENT_REF", "ref-admission-plain")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let plain: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(plain.get("admission_capability").is_none(), "{plain}");
    let _ = Command::new(&kill)
        .args(["--state-dir", state.to_str().unwrap()])
        .env("QUECTO_CONTAINER_CLI", cli)
        .env(
            "QUECTO_CONTAINER_ENVIRONMENT_ID",
            plain["environment_id"].as_str().unwrap(),
        )
        .status();
}
