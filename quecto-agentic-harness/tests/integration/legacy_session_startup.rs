//! Production startup of a legacy (pre-scoping) session (#2009, R2-H1): a
//! transcript with no `.home` is refused at startup — history is never
//! associated implicitly — and the refusal names the key and what the user
//! can do now. The transcript is preserved untouched.
use std::process::Command;

fn quecto(base: &std::path::Path, cwd: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_quecto"))
        .args(args)
        .env("QUECTO_BASE_DIR", base)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .current_dir(cwd)
        .output()
        .unwrap()
}

fn legacy_record(base: &std::path::Path, file: &str, key: &str) -> std::path::PathBuf {
    let sessions = base.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let path = sessions.join(file);
    std::fs::write(
        &path,
        format!(
            r#"{{"key":"{key}","messages":[{{"role":"user","content":"before scoping"}},{{"role":"assistant","content":"ok"}}]}}"#
        ),
    )
    .unwrap();
    path
}

#[test]
fn legacy_cli_default_and_named_sessions_are_refused_at_startup_with_actionable_text() {
    let base = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    std::fs::write(
        base.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-fake-test-key-1234","api_base":"http://127.0.0.1:1"}}}"#,
    )
    .unwrap();
    let default = legacy_record(base.path(), "cli_default.json", "cli:default");
    let named = legacy_record(base.path(), "cli_work.json", "cli:work");
    let default_bytes = std::fs::read(&default).unwrap();
    let named_bytes = std::fs::read(&named).unwrap();

    for (args, key) in [
        (vec!["agent", "-m", "hello"], "cli:default"),
        (vec!["agent", "-s", "work", "-m", "hello"], "cli:work"),
    ] {
        let output = quecto(base.path(), cwd.path(), &args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(1), "{stderr}");
        for expected in [
            &format!("session '{key}' cannot start here"),
            "legacy session requires explicit first association",
            "saved transcript was not changed",
        ] {
            assert!(
                stderr.contains(expected),
                "missing {expected:?} in: {stderr}"
            );
        }
        assert!(
            !stderr.contains("Cancel (open/fork/locate are unavailable)"),
            "the startup refusal is not the resume-picker refusal: {stderr}"
        );
    }
    assert_eq!(std::fs::read(&default).unwrap(), default_bytes);
    assert_eq!(std::fs::read(&named).unwrap(), named_bytes);
    assert!(!base.path().join("sessions/cli_default.home").exists());
    assert!(!base.path().join("sessions/cli_work.home").exists());

    // The way out the text names: a new name starts here (and fails only on
    // the unreachable provider, past the session open).
    let output = quecto(
        base.path(),
        cwd.path(),
        &["agent", "-s", "work2", "-m", "hello"],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("cannot start here"), "{stderr}");
    assert!(stderr.contains("Error:"), "{stderr}");
}
