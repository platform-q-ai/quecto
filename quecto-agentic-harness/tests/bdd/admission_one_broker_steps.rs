//! #2024 S3: the one host-wide admission broker, agent-operable. The systemd
//! user service installs and uninstalls idempotently behind a fake
//! `systemctl`; status addresses the global broker whatever the working
//! directory; and an unlisted slot binds to the default alias. The
//! real-process child-inherits and reset/restart recovery scenarios live in
//! `admission_recovery_steps`.
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use cucumber::{given, then, when};

use crate::QuectoWorld;

const LIMIT: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct OneBrokerState {
    temp: Option<tempfile::TempDir>,
    broker: Option<Child>,
    runtime: Option<tokio::runtime::Runtime>,
    systemctl_log: PathBuf,
    cli_out: String,
    cli_err: String,
    cli_code: i32,
}

impl std::fmt::Debug for OneBrokerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OneBrokerState")
    }
}

impl Drop for OneBrokerState {
    fn drop(&mut self) {
        if let Some(mut broker) = self.broker.take() {
            let _ = broker.kill();
            let _ = broker.wait();
        }
    }
}

fn state(world: &mut QuectoWorld) -> &mut OneBrokerState {
    &mut world.one_broker
}

fn quecto_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_quecto"))
}

fn base(world: &mut QuectoWorld) -> PathBuf {
    state(world).temp.as_ref().unwrap().path().to_path_buf()
}

fn authority_dir(base: &Path) -> PathBuf {
    base.join("authority")
}

fn admission_config(base: &Path, mock: &str) -> String {
    let authority = authority_dir(base);
    format!(
        r#"{{"providers":{{"openai":{{"api_key":"sk-test","api_base":"{mock}"}}}},
"agents":{{"defaults":{{"model":"openai-api/gpt-4o-mini","workspace":{workspace:?}}}}},
"admission":{{"directory":{dir:?},"groups":{{"g":{{"capacity":4,"reserve":0,"min_interval_ms":1,"queue_capacity":16,"queue_timeout_ms":5000,"attempt_timeout_ms":30000,"fallback_base_ms":50,"max_cooldown_ms":500}}}},"aliases":{{"acct":"g"}},"bindings":{{"openai-api":"acct"}}}}}}"#,
        workspace = base.join("workspace").to_string_lossy(),
        dir = authority.to_string_lossy(),
    )
}

fn qcmd(base: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(quecto_binary());
    command
        .args(args)
        .env("QUECTO_BASE_DIR", base)
        .env("HOME", base)
        .env("XDG_CONFIG_HOME", base.join(".config"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn start_mock() -> (String, tokio::runtime::Runtime) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let uri = rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(|request: &wiremock::Request| {
                let streaming = request
                    .body_json::<serde_json::Value>()
                    .ok()
                    .and_then(|body| body.get("stream").and_then(serde_json::Value::as_bool))
                    .unwrap_or(false);
                if streaming {
                    let chunk = serde_json::json!({
                        "choices": [{"index": 0, "delta": {"content": "DONE"}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 5, "completion_tokens": 1, "total_tokens": 6}
                    });
                    wiremock::ResponseTemplate::new(200).set_body_raw(
                        format!("data: {chunk}\n\ndata: [DONE]\n\n"),
                        "text/event-stream",
                    )
                } else {
                    wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "id": "chatcmpl-test",
                        "object": "chat.completion",
                        "choices": [{"index": 0, "message": {"role": "assistant", "content": "DONE"}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 5, "completion_tokens": 1, "total_tokens": 6}
                    }))
                }
            })
            .mount(&server)
            .await;
        let uri = server.uri();
        std::mem::forget(server);
        uri
    });
    (uri, rt)
}

fn start_broker(base: &Path, config: &Path) -> Child {
    let child = qcmd(
        base,
        &[
            "--config",
            config.to_str().unwrap(),
            "admission-broker",
            "run",
        ],
    )
    .spawn()
    .expect("spawn broker");
    let socket = authority_dir(base).join("client").join("admission.sock");
    let deadline = Instant::now() + LIMIT;
    while std::os::unix::net::UnixStream::connect(&socket).is_err() {
        assert!(Instant::now() < deadline, "broker never accepted");
        std::thread::sleep(Duration::from_millis(20));
    }
    child
}

#[given("a running admission broker over a mock provider")]
fn given_broker(world: &mut QuectoWorld) {
    let temp = tempfile::tempdir().unwrap();
    let base_path = temp.path().to_path_buf();
    std::fs::create_dir_all(base_path.join("workspace")).unwrap();
    state(world).temp = Some(temp);
    let (uri, rt) = start_mock();
    state(world).runtime = Some(rt);
    let config_path = base_path.join("config.json");
    std::fs::write(&config_path, admission_config(&base_path, &uri)).unwrap();
    let broker = start_broker(&base_path, &config_path);
    state(world).broker = Some(broker);
}

// ── systemd user service via a fake systemctl ─────────────────────────────

#[given("a fake systemctl on the PATH")]
fn given_fake_systemctl(world: &mut QuectoWorld) {
    let temp = tempfile::tempdir().unwrap();
    let base_path = temp.path().to_path_buf();
    state(world).temp = Some(temp);
    let bindir = base_path.join("bin");
    std::fs::create_dir_all(&bindir).unwrap();
    std::fs::create_dir_all(base_path.join("workspace")).unwrap();
    let log = base_path.join("systemctl.log");
    let script = bindir.join("systemctl");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    state(world).systemctl_log = log;
    // An admission-enabled global config so install resolves a real directory.
    let config_path = base_path.join("config.json");
    std::fs::write(
        &config_path,
        admission_config(&base_path, "http://127.0.0.1:1"),
    )
    .unwrap();
}

fn run_broker_cli(world: &mut QuectoWorld, args: &[&str]) {
    let base_path = base(world);
    let bindir = base_path.join("bin");
    let path = format!(
        "{}:{}",
        bindir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = qcmd(&base_path, args)
        .env("PATH", path)
        .output()
        .expect("run broker cli");
    state(world).cli_code = output.status.code().unwrap_or(-1);
    state(world).cli_out = String::from_utf8_lossy(&output.stdout).to_string();
    state(world).cli_err = String::from_utf8_lossy(&output.stderr).to_string();
}

#[when("the operator installs the broker service with an explicit config")]
fn when_install(world: &mut QuectoWorld) {
    let base_path = base(world);
    let config = base_path.join("config.json");
    run_broker_cli(
        world,
        &[
            "admission-broker",
            "install-service",
            "--config",
            config.to_str().unwrap(),
        ],
    );
}

#[then("a user unit runs the broker with that absolute config and restarts on failure")]
fn then_unit_written(world: &mut QuectoWorld) {
    let (code, err) = {
        let s = state(world);
        (s.cli_code, s.cli_err.clone())
    };
    assert_eq!(code, 0, "{err}");
    let unit = base(world).join(".config/systemd/user/quecto-admission-broker.service");
    let contents = std::fs::read_to_string(&unit).expect("unit file written");
    assert!(
        contents.contains("admission-broker run --config"),
        "{contents}"
    );
    assert!(contents.contains("Restart=on-failure"), "{contents}");
    assert!(contents.contains("WantedBy=default.target"), "{contents}");
}

#[then("systemctl was asked to reload, enable and start the unit")]
fn then_systemctl_called(world: &mut QuectoWorld) {
    let log = std::fs::read_to_string(&state(world).systemctl_log).unwrap_or_default();
    assert!(log.contains("--user daemon-reload"), "{log}");
    assert!(
        log.contains("--user enable --now quecto-admission-broker.service"),
        "{log}"
    );
}

#[when("the operator installs the broker service again")]
fn when_install_again(world: &mut QuectoWorld) {
    let _ = std::fs::write(&state(world).systemctl_log, "");
    when_install(world);
}

#[then("the second install does not rewrite the unchanged unit")]
fn then_no_rewrite(world: &mut QuectoWorld) {
    let (code, err) = {
        let s = state(world);
        (s.cli_code, s.cli_err.clone())
    };
    assert_eq!(code, 0, "{err}");
    let out = state(world).cli_out.clone();
    assert!(out.contains("already up to date"), "{out}");
}

#[when("the operator uninstalls the broker service")]
fn when_uninstall(world: &mut QuectoWorld) {
    let base_path = base(world);
    let config = base_path.join("config.json");
    run_broker_cli(
        world,
        &[
            "admission-broker",
            "uninstall-service",
            "--config",
            config.to_str().unwrap(),
        ],
    );
}

#[then("the unit is removed and systemctl disabled it")]
fn then_removed(world: &mut QuectoWorld) {
    let (code, err) = {
        let s = state(world);
        (s.cli_code, s.cli_err.clone())
    };
    assert_eq!(code, 0, "{err}");
    let unit = base(world).join(".config/systemd/user/quecto-admission-broker.service");
    assert!(!unit.exists(), "unit removed");
    let log = std::fs::read_to_string(&state(world).systemctl_log).unwrap_or_default();
    assert!(
        log.contains("--user disable --now quecto-admission-broker.service"),
        "{log}"
    );
}

#[when("the operator uninstalls the broker service again")]
fn when_uninstall_again(world: &mut QuectoWorld) {
    when_uninstall(world);
}

#[then("the uninstall reports nothing to remove")]
fn then_nothing_to_remove(world: &mut QuectoWorld) {
    let (code, err) = {
        let s = state(world);
        (s.cli_code, s.cli_err.clone())
    };
    assert_eq!(code, 0, "{err}");
    let out = state(world).cli_out.clone();
    assert!(out.contains("no unit to remove"), "{out}");
}

// ── cwd-independent status ────────────────────────────────────────────────

#[when("the operator runs status from an unrelated working directory")]
fn when_status_from_subdir(world: &mut QuectoWorld) {
    let base_path = base(world);
    let subdir = base_path.join("workspace");
    let mut command = qcmd(&base_path, &["admission-broker", "status"]);
    command.current_dir(&subdir);
    let output = command.output().expect("status");
    state(world).cli_code = output.status.code().unwrap_or(-1);
    state(world).cli_out = String::from_utf8_lossy(&output.stdout).to_string();
    state(world).cli_err = String::from_utf8_lossy(&output.stderr).to_string();
}

#[when("the operator runs status with an explicit --config from an unrelated working directory")]
fn when_status_explicit_config(world: &mut QuectoWorld) {
    let base_path = base(world);
    // The explicit file lives outside the base directory, so only `--config`
    // (extracted globally, before the subcommand) can be what addressed it.
    let explicit = base_path.join("elsewhere").join("explicit.json");
    std::fs::create_dir_all(explicit.parent().unwrap()).unwrap();
    std::fs::copy(base_path.join("config.json"), &explicit).unwrap();
    let mut command = qcmd(
        &base_path,
        &[
            "--config",
            explicit.to_str().unwrap(),
            "admission-broker",
            "status",
        ],
    );
    command.current_dir(base_path.join("workspace"));
    let output = command.output().expect("status");
    state(world).cli_code = output.status.code().unwrap_or(-1);
    state(world).cli_out = String::from_utf8_lossy(&output.stdout).to_string();
    state(world).cli_err = String::from_utf8_lossy(&output.stderr).to_string();
}

#[then("the status output names the global authority directory")]
fn then_status_names_dir(world: &mut QuectoWorld) {
    let (code, err) = {
        let s = state(world);
        (s.cli_code, s.cli_err.clone())
    };
    assert_eq!(code, 0, "{err}");
    let directory = authority_dir(&base(world));
    let status: serde_json::Value = serde_json::from_str(state(world).cli_out.trim())
        .unwrap_or_else(|_| panic!("status json: {}", state(world).cli_out));
    assert_eq!(
        status["directory"].as_str().unwrap(),
        directory.to_string_lossy()
    );
    assert_eq!(status["epoch"], 1);
}

#[when("the operator runs status for a directory with no broker")]
fn when_status_missing_dir(world: &mut QuectoWorld) {
    let base_path = base(world);
    let missing = base_path.join("no-such-authority");
    run_broker_cli(
        world,
        &[
            "admission-broker",
            "status",
            "--directory",
            missing.to_str().unwrap(),
        ],
    );
}

#[then("the status output reports that directory as not running")]
fn then_status_not_running(world: &mut QuectoWorld) {
    assert_eq!(state(world).cli_code, 1);
    assert!(
        state(world).cli_err.contains("not running"),
        "{}",
        state(world).cli_err
    );
}

// ── default binding ───────────────────────────────────────────────────────

#[given("an admission config whose only binding is the default alias")]
fn given_default_binding_config(world: &mut QuectoWorld) {
    let temp = tempfile::tempdir().unwrap();
    let base_path = temp.path().to_path_buf();
    std::fs::create_dir_all(base_path.join("workspace")).unwrap();
    state(world).temp = Some(temp);
    let authority = authority_dir(&base_path);
    let config = format!(
        r#"{{"providers":{{"openai":{{"api_key":"sk-test","api_base":"http://127.0.0.1:1"}}}},
"admission":{{"directory":{dir:?},"groups":{{"g":{{"capacity":4,"reserve":0,"min_interval_ms":1,"queue_capacity":16,"queue_timeout_ms":5000,"attempt_timeout_ms":30000,"fallback_base_ms":50,"max_cooldown_ms":500}}}},"aliases":{{"acct":"g"}},"bindings":{{"*":"acct"}}}}}}"#,
        dir = authority.to_string_lossy(),
    );
    std::fs::write(base_path.join("config.json"), config).unwrap();
}

#[when("the broker serves that config")]
fn when_broker_serves(world: &mut QuectoWorld) {
    let base_path = base(world);
    let config = base_path.join("config.json");
    let broker = start_broker(&base_path, &config);
    state(world).broker = Some(broker);
}

#[then("the broker reports the authority as running")]
fn then_broker_running(world: &mut QuectoWorld) {
    let base_path = base(world);
    let config = base_path.join("config.json");
    run_broker_cli(
        world,
        &[
            "--config",
            config.to_str().unwrap(),
            "admission-broker",
            "status",
        ],
    );
    let (code, err) = {
        let s = state(world);
        (s.cli_code, s.cli_err.clone())
    };
    assert_eq!(code, 0, "{err}");
    let status: serde_json::Value = serde_json::from_str(state(world).cli_out.trim()).unwrap();
    assert_eq!(status["epoch"], 1);
    assert!(status["groups"]["g"].is_object());
}
