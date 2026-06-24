//! Region-coverage tests for `cli` flag parsing, stderr redaction, and
//! socket-path validation. No real agent process or TTY is involved.

use super::*;

fn args(s: &str) -> Vec<String> {
    let mut v = vec!["quecto-tui".to_string()];
    if !s.is_empty() {
        v.extend(s.split_whitespace().map(String::from));
    }
    v
}

fn tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "quecto-tui-clicov-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ── parse_flags ──────────────────────────────────────────────────────

#[test]
fn parse_socket_and_no_sandbox() {
    let flags = parse_flags(&args("--socket /tmp/a.sock --no-sandbox"));
    assert_eq!(
        flags.socket_path.as_ref().unwrap().to_str().unwrap(),
        "/tmp/a.sock"
    );
    assert!(flags.no_sandbox);
}

#[test]
fn parse_unknown_flag_is_ignored() {
    let flags = parse_flags(&args("--bogus value --workflow"));
    assert!(flags.workflow);
    assert!(flags.socket_path.is_none());
}

#[test]
fn parse_trailing_socket_without_value_is_ignored() {
    // `--socket` at the very end has no value, so the guard skips it.
    let flags = parse_flags(&args("--socket"));
    assert!(flags.socket_path.is_none());
}

#[test]
fn parse_config_without_value_is_ignored() {
    let flags = parse_flags(&args("--config"));
    assert!(flags.config_path.is_none());
}

// ── stderr line bookkeeping ──────────────────────────────────────────

#[test]
fn truncate_short_line_unchanged() {
    assert_eq!(truncate_stderr_line("hello"), "hello");
}

#[test]
fn truncate_long_line_appends_ellipsis() {
    let long = "x".repeat(MAX_STARTUP_STDERR_LINE_CHARS + 50);
    let out = truncate_stderr_line(&long);
    assert!(out.ends_with('…'));
    assert!(out.chars().count() <= MAX_STARTUP_STDERR_LINE_CHARS + 1);
}

#[test]
fn remember_stderr_line_evicts_oldest_over_cap() {
    let mut lines = Vec::new();
    for i in 0..(MAX_STARTUP_STDERR_LINES + 5) {
        remember_stderr_line(&mut lines, &format!("line {i}"));
    }
    assert_eq!(lines.len(), MAX_STARTUP_STDERR_LINES);
    // Oldest lines were evicted; the last appended line remains.
    assert!(
        lines
            .last()
            .unwrap()
            .contains(&format!("line {}", MAX_STARTUP_STDERR_LINES + 4))
    );
    assert!(!lines.iter().any(|l| l == "line 0"));
}

// ── redaction ────────────────────────────────────────────────────────

#[test]
fn redact_named_secret_unquoted_value() {
    let out = redact_named_secret_values("api_key=supersecretvalue trailing");
    assert!(out.contains("[REDACTED]"));
    assert!(!out.contains("supersecretvalue"));
    assert!(out.contains("trailing"));
}

#[test]
fn redact_named_secret_quoted_value() {
    let out = redact_named_secret_values("\"access_token\": \"abc123\", next");
    assert!(out.contains("[REDACTED]"));
    assert!(!out.contains("abc123"));
    assert!(out.contains("next"));
}

#[test]
fn redact_named_secret_without_separator_is_left_alone() {
    let out = redact_named_secret_values("api_key is unset");
    assert_eq!(out, "api_key is unset");
}

#[test]
fn redact_bearer_token_after_keyword() {
    let out = redact_bearer_tokens("Authorization Bearer tok-12345");
    assert!(out.contains("[REDACTED]"));
    assert!(!out.contains("tok-12345"));
}

#[test]
fn redact_bearer_without_following_token_is_noop() {
    let out = redact_bearer_tokens("just some words");
    assert_eq!(out, "just some words");
}

#[test]
fn looks_like_secret_token_matches_known_prefixes() {
    assert!(looks_like_secret_token("sk-ant-abcdefghijklmnop"));
    assert!(looks_like_secret_token("ghp_abcdefghijklmnopqr"));
    assert!(!looks_like_secret_token("sk-short"));
    assert!(!looks_like_secret_token("plainwordnottoken"));
}

#[test]
fn redact_secret_tokens_strips_surrounding_punctuation() {
    let out = redact_secret_tokens("prefix \"sk-ant-abcdefghijklmnop\",");
    assert!(out.contains("[REDACTED]"));
    assert!(!out.contains("abcdefghijklmnop"));
}

#[test]
fn redact_stderr_line_combines_all_strategies() {
    let line = "Authorization: Bearer sk-ant-secrettokenvalue api_key=AIzaSecretValueHere";
    let out = redact_stderr_line(line);
    assert!(out.contains("[REDACTED]"));
    assert!(!out.contains("secrettokenvalue"));
}

// ── socket path validation ───────────────────────────────────────────

#[test]
fn socket_path_rejects_relative() {
    let err = validate_socket_path(Path::new("relative/agent.sock")).unwrap_err();
    assert!(err.contains("not absolute"));
}

#[test]
fn socket_path_rejects_nonexistent() {
    let err = validate_socket_path(Path::new("/tmp/does-not-exist-quecto.sock")).unwrap_err();
    assert!(err.contains("not accessible"));
}

#[test]
fn socket_path_rejects_regular_file() {
    let dir = tmp_dir("regfile");
    let file = dir.join("agent.sock");
    std::fs::write(&file, b"not a socket").unwrap();
    let err = validate_socket_path(&file).unwrap_err();
    assert!(err.contains("not a Unix socket"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn socket_path_rejects_symlink() {
    let dir = tmp_dir("symlink");
    let real = dir.join("real.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&real).unwrap();
    let link = dir.join("link.sock");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let err = validate_socket_path(&link).unwrap_err();
    assert!(err.contains("must not be a symlink"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn socket_path_accepts_socket_under_tmp() {
    let dir = tmp_dir("ok");
    let sock = dir.join("agent.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    assert!(validate_socket_path(&sock).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn allowed_socket_roots_includes_tmp() {
    let roots = canonical_allowed_socket_roots();
    assert!(!roots.is_empty());
}

#[test]
fn parse_system_without_value_is_ignored() {
    let flags = parse_flags(&args("--system"));
    assert!(flags.system_prompt.is_none());
}

#[test]
fn parse_no_workflow_before_workflow_allows_reenable() {
    let flags = parse_flags(&args("--workflow-guards --no-workflow --workflow"));
    assert!(flags.workflow);
    assert!(!flags.workflow_guards);
    assert!(!flags.workflow_disabled);
}

#[test]
fn redact_named_secret_handles_single_quotes_and_json_stop() {
    let out = redact_named_secret_values("refresh_token: 'secret-value' other=ok");
    assert!(out.contains("'[REDACTED]'"), "{out}");
    assert!(!out.contains("secret-value"));

    let out = redact_named_secret_values("{\"id_token\":abc123}");
    assert!(out.contains("[REDACTED]"), "{out}");
    assert!(!out.contains("abc123"));
}

#[test]
fn redact_value_after_name_handles_repeated_names() {
    let out = redact_named_secret_values("api_key=first access_token=second tail");
    assert_eq!(out.matches("[REDACTED]").count(), 2, "{out}");
    assert!(!out.contains("first"));
    assert!(!out.contains("second"));
    assert!(out.contains("tail"));
}

#[test]
fn allowed_socket_roots_ignores_relative_env_values() {
    // Exercise the pure filter directly so we never mutate the process
    // environment — `std::env::set_var` races with the many other tests that
    // read TMPDIR/XDG_RUNTIME_DIR under cargo's parallel runner.
    let roots = canonicalize_socket_roots(vec![
        PathBuf::from("relative-tmp"),
        PathBuf::from("relative-xdg"),
        PathBuf::from("/tmp"),
    ]);
    // Relative roots are dropped; the surviving roots are all absolute.
    assert!(roots.iter().all(|p| p.is_absolute()));
    assert!(
        !roots.iter().any(|p| p.ends_with("relative-tmp")),
        "relative TMPDIR value must be rejected: {roots:?}"
    );
    assert!(
        !roots.iter().any(|p| p.ends_with("relative-xdg")),
        "relative XDG_RUNTIME_DIR value must be rejected: {roots:?}"
    );
}
