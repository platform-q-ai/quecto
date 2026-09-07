use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn run_repl(args: &[&str], input: &str) -> std::process::Output {
    let binary = env!("CARGO_BIN_EXE_quecto");
    let mut child = Command::new(binary)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
    panic!("production REPL did not consume interactive input before timeout");
}

#[test]
fn auth_login_consumes_provider_choice_without_relocking_stdin() {
    let output = run_repl(&[], "auth login\ninvalid\nexit\n");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Choose a provider"), "{stdout}");
    assert!(stdout.contains("invalid choice 'invalid'"), "{stdout}");
}

#[test]
fn config_only_invocation_rejects_a_missing_explicit_config() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.json");
    let output = run_repl(&["--config", missing.to_str().unwrap()], "exit\n");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("config not found:"), "{stderr}");
}

#[test]
fn option_shaped_config_value_is_rejected() {
    let output = run_repl(&["--config", "--help"], "exit\n");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--config requires a path"), "{stderr}");
}

#[test]
fn config_only_invocation_uses_the_live_repl() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.json");
    std::fs::write(&config, "{}").unwrap();
    let output = run_repl(&["--config", config.to_str().unwrap()], "help\nexit\n");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Setup and configuration commands"),
        "{stdout}"
    );
}
