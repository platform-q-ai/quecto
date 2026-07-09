//! Region-coverage tests for `cli` flag parsing, stderr redaction, and
//! socket-path validation. No TTY is involved; the spawn-path wiring test
//! drives the real `spawn_agent` flow with a stand-in shell script.

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
    use crate::infrastructure::child_watch::{STDERR_TAIL_MAX_LINES, StderrTail};
    let tail = StderrTail::default();
    for i in 0..(STDERR_TAIL_MAX_LINES + 5) {
        remember_stderr_line(&tail, &format!("line {i}"));
    }
    let lines = tail.lines();
    assert_eq!(lines.len(), STDERR_TAIL_MAX_LINES);
    // Oldest lines were evicted; the last appended line remains.
    assert!(
        lines
            .last()
            .unwrap()
            .contains(&format!("line {}", STDERR_TAIL_MAX_LINES + 4))
    );
    assert!(!lines.iter().any(|l| l == "line 0"));
}

// ── post-startup stderr drain (#1047) ────────────────────────────────

#[tokio::test]
async fn stderr_drain_captures_lines_and_redacts() {
    use crate::infrastructure::child_watch::StderrTail;
    let tail = StderrTail::default();
    let input: &[u8] = b"plain line\napi_key=supersecretvalue\n";
    spawn_stderr_drain(tokio::io::BufReader::new(input), tail.clone());
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tail.lines().len() < 2 {
        assert!(tokio::time::Instant::now() < deadline, "drain must finish");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    let lines = tail.lines();
    assert_eq!(lines[0], "plain line");
    assert!(lines[1].contains("[REDACTED]"));
    assert!(!lines[1].contains("supersecretvalue"));
}

#[tokio::test]
async fn capped_line_reader_truncates_oversized_lines_and_keeps_reading() {
    let oversized = "a".repeat(MAX_DRAIN_STDERR_LINE_BYTES * 3);
    let input = format!("{oversized}\nnext line\n");
    let mut reader = tokio::io::BufReader::new(input.as_bytes());
    let mut line = String::new();

    let consumed = read_stderr_line_capped(&mut reader, &mut line)
        .await
        .expect("read oversized line");
    assert_eq!(consumed, oversized.len() + 1, "the whole line is consumed");
    assert_eq!(line.len(), MAX_DRAIN_STDERR_LINE_BYTES, "kept bytes capped");

    read_stderr_line_capped(&mut reader, &mut line)
        .await
        .expect("read following line");
    assert_eq!(line, "next line");

    let eof = read_stderr_line_capped(&mut reader, &mut line)
        .await
        .expect("read at EOF");
    assert_eq!(eof, 0, "EOF reads 0 consumed bytes");
}

/// #1051 final review: a multi-byte character split across two pipe reads
/// must survive intact — per-chunk lossy decoding turned it into U+FFFD.
/// The duplex capacity of 5 forces the split inside the 3-byte em-dash.
#[tokio::test]
async fn capped_line_reader_keeps_multibyte_char_split_across_reads() {
    use tokio::io::AsyncWriteExt;
    let (mut server, client) = tokio::io::duplex(5);
    let writer = tokio::spawn(async move {
        server
            .write_all("pre — post\n".as_bytes())
            .await
            .expect("write split line");
    });
    let mut reader = tokio::io::BufReader::new(client);
    let mut line = String::new();
    read_stderr_line_capped(&mut reader, &mut line)
        .await
        .expect("read split line");
    assert_eq!(line, "pre — post");
    writer.await.expect("writer task");
}

/// #1051 final review: a character sliced by the byte cap itself is dropped,
/// not replaced with U+FFFD, so capped diagnostics stay clean.
#[tokio::test]
async fn capped_line_reader_drops_char_sliced_by_the_cap() {
    let mut input = "a".repeat(MAX_DRAIN_STDERR_LINE_BYTES - 1);
    input.push('—'); // 3 bytes: the cap slices it after its first byte.
    input.push('\n');
    let mut reader = tokio::io::BufReader::new(input.as_bytes());
    let mut line = String::new();
    read_stderr_line_capped(&mut reader, &mut line)
        .await
        .expect("read capped line");
    assert_eq!(
        line.len(),
        MAX_DRAIN_STDERR_LINE_BYTES - 1,
        "the sliced character is dropped"
    );
    assert!(!line.contains('\u{FFFD}'));
}

/// #1051 final review (falsifiability): the PRODUCTION spawn path must wire
/// the post-startup stderr drain — reverting the `spawn_stderr_drain` call in
/// `spawn_agent` must fail this test, not just the manually-wired drain unit
/// tests. A stand-in script announces a real Unix socket, then keeps writing
/// to stderr; the drain spawned inside `spawn_agent_program` must capture it.
#[tokio::test]
async fn spawn_agent_wires_the_post_startup_stderr_drain() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tmp_dir("spawnwire");
    let sock = dir.join("agent.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&sock).expect("bind fake socket");
    let script = dir.join("fake-agent.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n\
             echo 'quecto-agent-socket: {}' >&2\n\
             echo 'post-startup panic line' >&2\n\
             sleep 30\n",
            sock.display()
        ),
    )
    .expect("write fake agent script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("mark script executable");

    let flags = parse_flags(&args(""));
    let (path, mut child, tail, protocol) = spawn_agent_program(script.to_str().unwrap(), &flags)
        .await
        .expect("spawn fake agent");
    assert_eq!(path, sock);
    // This fake agent announces no protocol line, so the spawn must report the
    // legacy (None) framing rather than inventing a version (#1059).
    assert_eq!(protocol, None);

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while !tail
        .lines()
        .iter()
        .any(|l| l.contains("post-startup panic line"))
    {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the drain wired by spawn_agent must capture post-announcement stderr"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    crate::infrastructure::process::terminate_child(
        &mut child,
        crate::infrastructure::process::TERMINATE_GRACE_MS,
    )
    .await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// #1059: the spawn path must PARSE the `quecto-agent-protocol: N` line the
/// agent emits before its socket announcement, so the caller can negotiate
/// framing. Reverting the parse to always-None would keep every client on
/// legacy NDJSON forever — this pins the parse.
#[tokio::test]
async fn spawn_agent_parses_the_protocol_version_announcement() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tmp_dir("spawnproto");
    let sock = dir.join("agent.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&sock).expect("bind fake socket");
    let script = dir.join("fake-agent.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\n\
             echo 'quecto-agent-protocol: {}' >&2\n\
             echo 'quecto-agent-socket: {}' >&2\n\
             sleep 30\n",
            quecto_line_io::PROTOCOL_VERSION,
            sock.display()
        ),
    )
    .expect("write fake agent script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
        .expect("mark script executable");

    let flags = parse_flags(&args(""));
    let (_path, mut child, _tail, protocol) = spawn_agent_program(script.to_str().unwrap(), &flags)
        .await
        .expect("spawn fake agent");
    assert_eq!(
        protocol,
        Some(quecto_line_io::PROTOCOL_VERSION),
        "spawn must parse the announced protocol version"
    );

    crate::infrastructure::process::terminate_child(
        &mut child,
        crate::infrastructure::process::TERMINATE_GRACE_MS,
    )
    .await;
    let _ = std::fs::remove_dir_all(&dir);
}

/// #1059: the framing decision — frames for an announced v2+, legacy NDJSON for
/// an agent that announced nothing. Pins `should_speak_frames` so a revert that
/// always picks one framing is caught.
#[test]
fn should_speak_frames_negotiates_on_announced_version() {
    assert!(
        should_speak_frames(Some(quecto_line_io::PROTOCOL_VERSION)),
        "an agent announcing the current protocol version speaks frames"
    );
    assert!(
        should_speak_frames(Some(quecto_line_io::PROTOCOL_VERSION + 1)),
        "a newer protocol version still speaks frames"
    );
    assert!(
        !should_speak_frames(Some(quecto_line_io::PROTOCOL_VERSION - 1)),
        "an older announced version falls back to legacy NDJSON"
    );
    assert!(
        !should_speak_frames(None),
        "an agent that announced no version is treated as legacy NDJSON"
    );
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
