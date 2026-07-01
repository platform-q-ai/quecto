//! Unit tests for quecto-tui CLI flag parsing (extracted from cli.rs to respect the 750-line cap).

use super::*;

fn args(s: &str) -> Vec<String> {
    let mut v = vec!["quecto-tui".to_string()];
    if !s.is_empty() {
        v.extend(s.split_whitespace().map(String::from));
    }
    v
}

#[test]
fn parse_workflow_flags() {
    let flags = parse_flags(&args("--workflow --workflow-guards"));
    assert!(flags.workflow);
    assert!(flags.workflow_guards);
}

#[test]
fn parse_no_workflow_clears_both() {
    let flags = parse_flags(&args("--workflow --workflow-guards --no-workflow"));
    assert!(!flags.workflow);
    assert!(!flags.workflow_guards);
    assert!(flags.workflow_disabled);
}

#[test]
fn build_args_forward_no_workflow_to_owned_agent() {
    let flags = parse_flags(&args("--no-workflow"));
    let agent_args = build_agent_args(&flags);
    assert!(agent_args.contains(&"--no-workflow".to_string()));
    assert!(!agent_args.contains(&"--workflow".to_string()));
    assert!(!agent_args.contains(&"--workflow-guards".to_string()));
}

#[test]
fn parse_disable_tool_repeatable() {
    let flags = parse_flags(&args("--disable-tool write --disable-tool edit"));
    assert_eq!(flags.disable_tools, vec!["write", "edit"]);
}

#[test]
fn parse_trailing_disable_tool_without_value_is_dropped() {
    // A trailing `--disable-tool` with no value must not capture a bogus tool name;
    // it is dropped (with a stderr warning) rather than silently consumed.
    let flags = parse_flags(&args("--disable-tool"));
    assert!(flags.disable_tools.is_empty());
}

#[test]
fn build_args_forward_disable_tool_to_owned_agent() {
    let flags = parse_flags(&args("--disable-tool write --disable-tool edit"));
    let agent_args = build_agent_args(&flags);
    let names: Vec<&str> = agent_args
        .iter()
        .zip(agent_args.iter().skip(1))
        .filter(|(a, _)| a.as_str() == "--disable-tool")
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(names, vec!["write", "edit"]);
}

#[test]
fn build_args_omit_disable_tool_when_none() {
    let flags = parse_flags(&args(""));
    let agent_args = build_agent_args(&flags);
    assert!(!agent_args.contains(&"--disable-tool".to_string()));
}

#[test]
fn parse_config_and_system() {
    let flags = parse_flags(&args("--config ./repo/config.json --system hello"));
    assert_eq!(
        flags.config_path.unwrap().to_str().unwrap(),
        "./repo/config.json"
    );
    assert_eq!(flags.system_prompt.as_deref(), Some("hello"));
}

fn write_tmp(name: &str, body: &str) -> String {
    let dir =
        std::env::temp_dir().join(format!("quecto-tui-sysfile-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("prompt.txt");
    std::fs::write(&path, body).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn system_file_is_read_as_prompt() {
    let p = write_tmp("read", "FROM FILE PROMPT");
    let flags = parse_flags(&["quecto-tui".into(), "--system-file".into(), p]);
    assert_eq!(flags.system_prompt.as_deref(), Some("FROM FILE PROMPT"));
}

#[test]
fn system_literal_wins_over_system_file_either_order() {
    let p = write_tmp("prec", "FILE");
    let before = parse_flags(&[
        "quecto-tui".into(),
        "--system".into(),
        "LIT".into(),
        "--system-file".into(),
        p.clone(),
    ]);
    assert_eq!(before.system_prompt.as_deref(), Some("LIT"));
    let after = parse_flags(&[
        "quecto-tui".into(),
        "--system-file".into(),
        p,
        "--system".into(),
        "LIT".into(),
    ]);
    assert_eq!(after.system_prompt.as_deref(), Some("LIT"));
}

#[test]
fn missing_system_file_leaves_prompt_unset() {
    let flags = parse_flags(&[
        "quecto-tui".into(),
        "--system-file".into(),
        "/nonexistent/quecto-tui/system-prompt".into(),
    ]);
    assert!(flags.system_prompt.is_none());
}

#[test]
fn workflow_without_system_gets_default_prompt() {
    let mut flags = parse_flags(&args("--workflow"));
    apply_workflow_defaults(&mut flags);
    assert!(flags.system_prompt.is_some());
    assert!(flags.system_prompt.unwrap().contains("workflow"));
}

#[test]
fn workflow_with_explicit_system_keeps_it() {
    let mut flags = parse_flags(&args("--workflow --system custom"));
    apply_workflow_defaults(&mut flags);
    assert_eq!(flags.system_prompt.as_deref(), Some("custom"));
}

#[test]
fn no_workflow_no_default_prompt() {
    let mut flags = parse_flags(&args(""));
    apply_workflow_defaults(&mut flags);
    assert!(flags.system_prompt.is_none());
}

#[test]
fn startup_failure_includes_agent_stderr_context() {
    let message = format_agent_startup_failure(
        "agent exited before announcing socket",
        &["no LLM providers configured (set an API key or run 'quecto auth login')".to_string()],
    );

    assert!(message.contains("agent exited before announcing socket"));
    assert!(message.contains("Agent stderr:"));
    assert!(message.contains("no LLM providers configured"));
}

#[test]
fn startup_failure_without_stderr_keeps_original_reason() {
    let message = format_agent_startup_failure("timeout waiting for agent socket path", &[]);
    assert_eq!(message, "timeout waiting for agent socket path");
}

// #808: cold-binary first launch after install must not time out at 10s.
#[test]
fn agent_socket_deadline_is_thirty_seconds() {
    assert_eq!(
        AGENT_SOCKET_DEADLINE,
        std::time::Duration::from_secs(30),
        "spawn->socket readiness deadline must be 30s to cover a cold-binary first launch"
    );
}

#[test]
fn agent_starting_status_names_the_wait() {
    let status = agent_starting_status();
    assert!(
        status.to_lowercase().contains("starting agent"),
        "the readiness wait must surface a 'starting agent' status: {status:?}"
    );
}

#[test]
fn agent_socket_timeout_message_is_actionable() {
    let message = agent_socket_timeout_message();
    // Names the cold-start / first-run-after-install cause.
    let lower = message.to_lowercase();
    assert!(
        lower.contains("cold") || lower.contains("first run") || lower.contains("first launch"),
        "timeout message must name the cold-binary / first-run cause: {message:?}"
    );
    // Names the warm remedy and the retry option.
    assert!(
        message.contains("quecto --version"),
        "timeout message must suggest running `quecto --version` to warm the binary: {message:?}"
    );
    assert!(
        lower.contains("retry") || lower.contains("try again"),
        "timeout message must mention retrying: {message:?}"
    );
}

#[test]
fn agent_socket_timeout_message_flows_through_failure_formatter() {
    let message =
        format_agent_startup_failure(&agent_socket_timeout_message(), &["boom".to_string()]);
    assert!(message.contains("quecto --version"));
    assert!(message.contains("Agent stderr:"));
    assert!(message.contains("boom"));
}

#[test]
fn stderr_context_redacts_common_secret_shapes() {
    let mut lines = Vec::new();
    remember_stderr_line(
        &mut lines,
        "Authorization: Bearer sk-ant-secret-token api_key=sk-test-secret-token",
    );

    assert_eq!(lines.len(), 1);
    assert!(lines[0].contains("[REDACTED]"));
    assert!(!lines[0].contains("sk-ant-secret-token"));
    assert!(!lines[0].contains("sk-test-secret-token"));
}

#[test]
fn socket_path_rejects_parent_dir_components() {
    let path = PathBuf::from("/tmp/../var/run/quecto.sock");
    let err = validate_socket_path(&path).unwrap_err();
    assert!(err.contains("must not contain '..'"));
}

#[test]
fn socket_path_accepts_real_socket_under_tmp() {
    let dir = std::env::temp_dir().join(format!(
        "quecto-tui-cli-test-{}-{}",
        std::process::id(),
        unique_test_suffix()
    ));
    std::fs::create_dir(&dir).expect("create temp socket dir");
    let socket = dir.join("agent.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).expect("bind test socket");

    let result = validate_socket_path(&socket);

    drop(listener);
    let _ = std::fs::remove_file(&socket);
    let _ = std::fs::remove_dir(&dir);
    assert!(
        result.is_ok(),
        "expected socket path to validate: {result:?}"
    );
}

fn unique_test_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time after epoch")
        .as_nanos()
}
