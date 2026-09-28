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
    if let Err(error) = child.stdin.take().unwrap().write_all(input.as_bytes()) {
        // Invalid arguments may close stdin before the parent is scheduled.
        // Each scenario still verifies the actual exit status and diagnostics.
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe, "{error}");
    }
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
    assert!(!output.status.success());
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

// Only this test binds the provider callback listeners' fixed ports
// (127.0.0.1:1455 and :56121); nothing else in the suite listens there.
// piped_repl_oauth_fails_promptly_for_both_providers takes the same serial
// key, although its non-interactive REPL skips the browser callback and binds
// nothing, so a future change that makes it bind cannot race this test.
#[test]
#[serial_test::serial(oauth_callback_ports)]
fn standalone_oauth_with_redirected_stdin_starts_browser_callbacks() {
    use std::io::Read;
    use std::net::TcpStream;

    for (provider, address, path) in [
        ("openai", "127.0.0.1:1455", "/auth/callback"),
        ("xai", "127.0.0.1:56121", "/callback"),
    ] {
        let base = tempfile::tempdir().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_quecto"))
            .args(["auth", "login", "--provider", provider])
            .env("QUECTO_BASE_DIR", base.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // A real callback listener rejects missing state without contacting a
        // provider. The request goes out in one write_all: the listener reads
        // the whole request head before answering (PR #2309), so how the bytes
        // are split no longer matters, but one write keeps the test simple.
        let request = format!(
            "GET {path}?code=test HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut response = String::new();
        let mut last_error: Option<std::io::Error> = None;
        // Retrying is cheap insurance only: connect fails until the child has
        // bound its port, and any other failure is kept for the assert message.
        while Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            match TcpStream::connect(address) {
                Ok(mut stream) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    response.clear();
                    let exchanged = stream
                        .write_all(request.as_bytes())
                        .and_then(|()| stream.read_to_string(&mut response));
                    match exchanged {
                        Ok(_) if response.starts_with("HTTP/1.1 ") => break,
                        Ok(_) => {}
                        Err(error) => last_error = Some(error),
                    }
                }
                Err(error) => last_error = Some(error),
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // The child must still be running: a response from some other process
        // listening on the fixed port must not pass this test.
        let child_status = child.try_wait().unwrap();
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        assert!(
            child_status.is_none(),
            "{provider}: login exited ({child_status:?}) before answering; response: \
             {response:?}; last I/O error: {last_error:?}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            response.starts_with("HTTP/1.1 400"),
            "{provider}: callback listener unavailable: {response:?}; last I/O error: \
             {last_error:?}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
#[serial_test::serial(oauth_callback_ports)]
fn piped_repl_oauth_fails_promptly_for_both_providers() {
    for provider in ["openai", "xai"] {
        let output = run_repl(
            &[],
            &format!("auth login --provider {provider}\n\nhelp\nexit\n"),
        );
        assert!(!output.status.success(), "{provider}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("could not extract authorization code"),
            "{stdout}"
        );
        assert!(
            stdout.contains("Setup and configuration commands"),
            "{stdout}"
        );
    }
}
