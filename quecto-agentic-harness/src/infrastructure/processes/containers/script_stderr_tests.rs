use super::*;

/// A script run through `bash` rather than executed directly: a file
/// written and executed by a multi-threaded test process can race a
/// concurrent fork holding it open (ETXTBSY).
fn script(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    let path = dir.join("script.sh");
    std::fs::write(&path, format!("{body}\n")).unwrap();
    path
}

fn bash(path: &std::path::Path) -> std::process::Command {
    let mut cmd = std::process::Command::new("bash");
    cmd.arg(path);
    cmd
}

#[test]
fn sanitise_strips_escapes_and_control_characters_but_keeps_newlines() {
    assert_eq!(
        sanitise("\u{1b}[31mred\u{1b}[0m\r\nnext\ttab\u{7}bell\u{1b}]0;title\u{7}end\n"),
        "red\nnext tabbellend"
    );
    assert_eq!(sanitise("a\u{1b}]0;t\u{1b}\\b"), "ab");
    assert_eq!(sanitise("a\u{1b}]0;unterminated\nnext"), "a\nnext");
    assert_eq!(sanitise("a\u{1b}(Bb\u{1b}=c"), "abc");
    assert_eq!(sanitise("\u{1b}[\nfatal: x"), "fatal: x");
    assert_eq!(sanitise("\u{1b}[1;31mred"), "red");
    assert_eq!(sanitise("  plain  "), "plain");
    assert_eq!(sanitise("\u{1b}"), "");
    assert_eq!(sanitise("\u{1b}[1;3"), "");
    assert_eq!(sanitise("\u{1b}[Kkept"), "kept");
}

#[test]
fn sanitise_redacts_the_userinfo_of_an_echoed_url() {
    assert_eq!(
        sanitise("create: --repo https://user:ghp_secret@host/x/y is unreachable\n"),
        "create: --repo https://***@host/x/y is unreachable"
    );
    assert_eq!(sanitise("https://host/x/y"), "https://host/x/y");
}

#[test]
fn failure_message_keeps_the_status_prefix_and_appends_the_tail_when_present() {
    let status = std::process::Command::new("false").status().unwrap();
    assert_eq!(
        failure_message("create", &status, ""),
        format!("script-managed create failed with status {status}")
    );
    assert_eq!(
        failure_message("exec", &status, "image x is not present"),
        format!("script-managed exec failed with status {status}: image x is not present")
    );
}

#[tokio::test]
async fn async_runner_keeps_stdout_whole_and_only_the_stderr_tail() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = script(
        dir.path(),
        "printf '{\"ok\":true}'\nhead -c 9000 /dev/zero | tr '\\0' x >&2\nprintf '\\nLAST\\n' >&2\nexit 3",
    );
    let output = run_capturing_stderr_tail(
        tokio::process::Command::from(bash(&path)),
        ScriptStdout::Result,
    )
    .await
    .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(output.stdout, b"{\"ok\":true}");
    assert!(output.stderr_tail.len() <= STDERR_TAIL_CAPACITY);
    assert!(
        output.stderr_tail.ends_with("LAST"),
        "{}",
        output.stderr_tail
    );
    assert!(
        output.failure_message("create").starts_with(&format!(
            "script-managed create failed with status {}: xxx",
            output.status
        )),
        "{}",
        output.failure_message("create")
    );
}

#[test]
fn sync_runner_bounds_the_tail_and_the_wall_clock() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = script(
        dir.path(),
        "printf out\nhead -c 9000 /dev/zero | tr '\\0' y >&2\nprintf '\\n\\033[1mLAST\\033[0m\\n' >&2\nexit 2",
    );
    let output =
        run_sync_capturing_stderr_tail(bash(&path), ScriptStdout::Result, Duration::from_secs(10))
            .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(output.stdout, b"out");
    assert!(output.stderr_tail.len() <= STDERR_TAIL_CAPACITY);
    assert!(
        output.stderr_tail.ends_with("LAST"),
        "{}",
        output.stderr_tail
    );
    assert!(!output.stderr_tail.contains('\u{1b}'));

    let slow = script(dir.path(), "echo 'still going' >&2\nsleep 5");
    let error = run_sync_capturing_stderr_tail(
        bash(&slow),
        ScriptStdout::Discard,
        Duration::from_millis(300),
    )
    .unwrap_err();
    assert!(error.contains("timed out"), "{error}");
    assert!(error.ends_with(": still going"), "{error}");

    // A grandchild holding the pipes open costs the grace, not liveness.
    let holder = script(dir.path(), "sleep 30 &\necho 'parent done' >&2\nexit 0");
    let started = std::time::Instant::now();
    let output = run_sync_capturing_stderr_tail(
        bash(&holder),
        ScriptStdout::Discard,
        Duration::from_secs(10),
    )
    .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stderr_tail, "parent done");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );

    let missing = run_sync_capturing_stderr_tail(
        std::process::Command::new("/definitely/not/a/script"),
        ScriptStdout::Result,
        Duration::from_secs(1),
    )
    .unwrap_err();
    assert!(
        missing.starts_with("failed to invoke script: "),
        "{missing}"
    );
}
