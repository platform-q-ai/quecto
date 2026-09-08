//! P3 real multi-process proof: an authority process, independent root agent
//! processes and a descendant context share one bound against a fake HTTP
//! provider that measures peak concurrency. No paid traffic, no Docker.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use quecto::domain::inference_admission::WorkloadClass;
use quecto::infrastructure::admission::{AdminConnection, AuthorityConnection, AuthorityDirectory};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const LIMIT: Duration = Duration::from_secs(30);

fn quecto_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_quecto"))
}

/// Minimal OpenAI-compatible chat endpoint that holds each request for
/// `hold` and records in-flight peak and total counts.
struct FakeProvider {
    base: String,
    peak: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    inflight: Arc<AtomicUsize>,
}

async fn fake_provider(hold: Duration) -> FakeProvider {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let peak = Arc::new(AtomicUsize::new(0));
    let total = Arc::new(AtomicUsize::new(0));
    let inflight = Arc::new(AtomicUsize::new(0));
    let (p, t, i) = (peak.clone(), total.clone(), inflight.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let (p, t, i) = (p.clone(), t.clone(), i.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let body_start;
                loop {
                    let n = stream.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        body_start = pos + 4;
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&buf[..body_start]).to_string();
                let length = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                    })
                    .unwrap_or(0);
                while buf.len() < body_start + length {
                    let n = stream.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let body = String::from_utf8_lossy(&buf[body_start..]).to_string();
                let streaming = body.contains("\"stream\":true");
                let now = i.fetch_add(1, Ordering::SeqCst) + 1;
                p.fetch_max(now, Ordering::SeqCst);
                t.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(hold).await;
                i.fetch_sub(1, Ordering::SeqCst);
                let response = if streaming {
                    let events = concat!(
                        "data: {\"id\":\"fake\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"admitted-reply\"},\"finish_reason\":null}]}\n\n",
                        "data: {\"id\":\"fake\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n",
                        "data: [DONE]\n\n"
                    );
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        events.len(),
                        events
                    )
                } else {
                    let json = "{\"id\":\"fake\",\"object\":\"chat.completion\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"admitted-reply\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1}}";
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        json.len(),
                        json
                    )
                };
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    FakeProvider {
        base: format!("http://127.0.0.1:{port}"),
        peak,
        total,
        inflight,
    }
}

fn write_config(dir: &Path, provider_base: &str, admission: Option<(usize, u64)>) -> PathBuf {
    let authority = dir.join("authority");
    let admission = match admission {
        Some((capacity, queue_timeout_ms)) => format!(
            r#","admission":{{"directory":{dir:?},"groups":{{"g":{{"capacity":{capacity},"reserve":0,"min_interval_ms":1,"queue_capacity":16,"queue_timeout_ms":{queue_timeout_ms},"attempt_timeout_ms":120000,"fallback_base_ms":100,"max_cooldown_ms":10000}}}},"aliases":{{"acct":"g"}},"bindings":{{"fake":"acct"}},"max_scopes":64,"terminal_capacity":256}}"#,
            dir = authority.to_string_lossy()
        ),
        None => String::new(),
    };
    let config = format!(
        r#"{{"agents":{{"defaults":{{"workspace":{ws:?},"model":"fake/test-model"}}}},"providers":{{"openai_compatible":{{"endpoints":[{{"prefix":"fake","api_key":"fake-key","api_base":"{provider_base}","allow_remote_http":true}}]}}}}{admission}}}"#,
        ws = dir.join("workspace").to_string_lossy()
    );
    std::fs::create_dir_all(dir.join("workspace")).unwrap();
    let path = dir.join("config.json");
    std::fs::write(&path, config).unwrap();
    path
}

struct Proc(Child);
impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn quecto(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(quecto_binary());
    command
        .args(args)
        .env("QUECTO_BASE_DIR", dir)
        .env("HOME", dir)
        .env("XDG_RUNTIME_DIR", dir.join("run"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    std::fs::create_dir_all(dir.join("run")).unwrap();
    command
}

fn start_broker(dir: &Path, config: &Path) -> Proc {
    let child = quecto(
        dir,
        &[
            "--config",
            config.to_str().unwrap(),
            "admission-broker",
            "run",
        ],
    )
    .spawn()
    .expect("spawn broker");
    // A stale socket file from a killed authority may still exist: readiness
    // is a successful connection, not the file's presence.
    let socket = dir.join("authority").join("client").join("admission.sock");
    let deadline = Instant::now() + LIMIT;
    while std::os::unix::net::UnixStream::connect(&socket).is_err() {
        assert!(
            Instant::now() < deadline,
            "authority never accepted on {}",
            socket.display()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    Proc(child)
}

fn root_agent(dir: &Path, config: &Path, extra: &[&str]) -> Child {
    let mut args = vec![
        "--config",
        config.to_str().unwrap(),
        "agent",
        "-m",
        "hello",
        "--no-session",
    ];
    args.extend_from_slice(extra);
    quecto(dir, &args).spawn().expect("spawn agent")
}

fn finish(child: Child) -> (bool, String, String) {
    let output = child.wait_with_output().unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn independent_root_processes_share_the_configured_bound() {
    let temp = tempfile::tempdir().unwrap();
    let provider = fake_provider(Duration::from_millis(600)).await;
    let config = write_config(temp.path(), &provider.base, Some((2, 20_000)));
    let _broker = start_broker(temp.path(), &config);
    let roots: Vec<Child> = (0..3)
        .map(|_| root_agent(temp.path(), &config, &[]))
        .collect();
    let results: Vec<_> =
        tokio::task::spawn_blocking(move || roots.into_iter().map(finish).collect::<Vec<_>>())
            .await
            .unwrap();
    for (ok, stdout, stderr) in &results {
        assert!(*ok, "root agent failed: {stderr}");
        assert!(
            stdout.contains("admitted-reply"),
            "root did not receive the reply: {stdout}"
        );
    }
    assert_eq!(
        provider.total.load(Ordering::SeqCst),
        3,
        "every root made exactly one attempt"
    );
    assert!(
        provider.peak.load(Ordering::SeqCst) <= 2,
        "peak {} exceeded configured capacity 2",
        provider.peak.load(Ordering::SeqCst)
    );
    assert_eq!(provider.inflight.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_admission_the_fake_provider_observes_the_unbounded_burst() {
    let temp = tempfile::tempdir().unwrap();
    let provider = fake_provider(Duration::from_millis(600)).await;
    let config = write_config(temp.path(), &provider.base, None);
    let roots: Vec<Child> = (0..3)
        .map(|_| root_agent(temp.path(), &config, &[]))
        .collect();
    let results: Vec<_> =
        tokio::task::spawn_blocking(move || roots.into_iter().map(finish).collect::<Vec<_>>())
            .await
            .unwrap();
    for (ok, _, stderr) in &results {
        assert!(*ok, "root agent failed: {stderr}");
    }
    assert_eq!(
        provider.peak.load(Ordering::SeqCst),
        3,
        "control: the oracle sees the burst"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn descendant_context_waits_behind_its_root_and_a_forged_context_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let provider = fake_provider(Duration::from_millis(50)).await;
    let config = write_config(temp.path(), &provider.base, Some((1, 20_000)));
    let _broker = start_broker(temp.path(), &config);
    let dir = AuthorityDirectory::open(&temp.path().join("authority")).unwrap();
    let root = AuthorityConnection::connect(&dir.client_socket())
        .await
        .unwrap();
    let credential = root
        .register_root(WorkloadClass::Interactive)
        .await
        .unwrap();
    root.bind(credential).await.unwrap();
    let child = root.register_child().await.unwrap();
    let context = temp.path().join("child-admission.json");
    quecto::infrastructure::admission::write_admission_context(
        &context,
        &dir.client_socket(),
        &child,
    )
    .unwrap();
    let permit = root.gate("acct").unwrap().acquire().await.unwrap();
    let agent = root_agent(
        temp.path(),
        &config,
        &["--admission-context", context.to_str().unwrap()],
    );
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(
        provider.total.load(Ordering::SeqCst),
        0,
        "descendant waits while the root holds C=1"
    );
    let admin = AdminConnection::connect(&dir.admin_socket()).await.unwrap();
    let status = admin.inspect().await.unwrap();
    let g = status.groups[&quecto::domain::inference_admission::GroupId::new("g").unwrap()];
    assert_eq!(
        (g.active, g.queued),
        (1, 1),
        "descendant is queued, not bypassing"
    );
    permit.finish(quecto::domain::inference_admission::Feedback::Success);
    let (ok, stdout, stderr) = tokio::task::spawn_blocking(move || finish(agent))
        .await
        .unwrap();
    assert!(ok, "descendant failed: {stderr}");
    assert!(stdout.contains("admitted-reply"));
    assert_eq!(provider.total.load(Ordering::SeqCst), 1);

    let forged = temp.path().join("forged.json");
    let mut bogus = child.clone();
    bogus.token = "forged".into();
    quecto::infrastructure::admission::write_admission_context(
        &forged,
        &dir.client_socket(),
        &bogus,
    )
    .unwrap();
    let agent = root_agent(
        temp.path(),
        &config,
        &["--admission-context", forged.to_str().unwrap()],
    );
    let (ok, _, stderr) = tokio::task::spawn_blocking(move || finish(agent))
        .await
        .unwrap();
    assert!(!ok, "forged capability must fail before inference");
    assert!(
        stderr.contains("admission"),
        "failure names admission: {stderr}"
    );
    assert_eq!(
        provider.total.load(Ordering::SeqCst),
        1,
        "no bypass attempt was made"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigkilled_client_leaves_uncertain_occupancy_until_operator_reset() {
    let temp = tempfile::tempdir().unwrap();
    let provider = fake_provider(Duration::from_secs(20)).await;
    let config = write_config(temp.path(), &provider.base, Some((1, 2_000)));
    let _broker = start_broker(temp.path(), &config);
    let victim = root_agent(temp.path(), &config, &[]);
    let deadline = Instant::now() + LIMIT;
    while provider.inflight.load(Ordering::SeqCst) == 0 {
        assert!(
            Instant::now() < deadline,
            "victim never reached the provider"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let pid = victim.id();
    assert!(
        Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    let _ = tokio::task::spawn_blocking(move || finish(victim)).await;
    let dir = AuthorityDirectory::open(&temp.path().join("authority")).unwrap();
    let admin = AdminConnection::connect(&dir.admin_socket()).await.unwrap();
    let group = quecto::domain::inference_admission::GroupId::new("g").unwrap();
    let deadline = Instant::now() + LIMIT;
    loop {
        let status = admin.inspect().await.unwrap();
        if status.groups[&group].uncertain == 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "authority never marked the killed client's attempt uncertain"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let (ok, _, stderr) = tokio::task::spawn_blocking({
        let (d, c) = (temp.path().to_path_buf(), config.clone());
        move || finish(root_agent(&d, &c, &[]))
    })
    .await
    .unwrap();
    assert!(!ok, "quarantined group must fail the next root explicitly");
    assert!(stderr.contains("admission"), "{stderr}");
    let output = quecto(
        temp.path(),
        &[
            "--config",
            config.to_str().unwrap(),
            "admission-broker",
            "reset",
        ],
    )
    .output()
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status = admin.inspect().await.unwrap();
    assert_eq!(status.groups[&group].uncertain, 0);
    assert_eq!(status.epoch, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigkilled_authority_restarts_with_orphans_and_does_not_kill_clients() {
    let temp = tempfile::tempdir().unwrap();
    let provider = fake_provider(Duration::from_secs(20)).await;
    let config = write_config(temp.path(), &provider.base, Some((2, 2_000)));
    let broker = start_broker(temp.path(), &config);
    let socket = temp.path().join("run").join("root.sock");
    let mut uds_root = quecto(
        temp.path(),
        &[
            "--config",
            config.to_str().unwrap(),
            "agent",
            "--mode",
            "uds",
            "--no-session",
            "--socket",
            socket.to_str().unwrap(),
        ],
    )
    .spawn()
    .unwrap();
    let deadline = Instant::now() + LIMIT;
    while !socket.exists() {
        assert!(
            uds_root.try_wait().unwrap().is_none(),
            "uds root exited early"
        );
        assert!(Instant::now() < deadline, "uds root never became ready");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let holder = root_agent(temp.path(), &config, &[]);
    let deadline = Instant::now() + LIMIT;
    while provider.inflight.load(Ordering::SeqCst) == 0 {
        assert!(
            Instant::now() < deadline,
            "holder never reached the provider"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let broker_pid = broker.0.id();
    assert!(
        Command::new("kill")
            .args(["-KILL", &broker_pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    drop(broker);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        uds_root.try_wait().unwrap().is_none(),
        "authority death must not kill unrelated sessions"
    );
    let _restarted = start_broker(temp.path(), &config);
    let output = quecto(
        temp.path(),
        &[
            "--config",
            config.to_str().unwrap(),
            "admission-broker",
            "status",
        ],
    )
    .output()
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let status: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(status["epoch"], 1, "restart keeps the epoch");
    assert_eq!(
        status["groups"]["g"]["uncertain"], 1,
        "outstanding attempt is orphaned, not forgotten"
    );
    assert_eq!(status["groups"]["g"]["active"], 1);
    let _ = uds_root.kill();
    let _ = uds_root.wait();
    let _ = tokio::task::spawn_blocking(move || finish(holder)).await;
}
