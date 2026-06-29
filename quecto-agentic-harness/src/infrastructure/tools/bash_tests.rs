use super::*;
use tempfile::TempDir;

fn test_exec(restrict: bool) -> (ExecTool, TempDir) {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), restrict);
    let tool = ExecTool::new(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox));
    (tool, tmp)
}

#[tokio::test]
async fn test_exec_echo() {
    let (tool, _tmp) = test_exec(false);
    let result = tool.execute(r#"{"command": "echo hello"}"#).await.unwrap();
    assert!(!result.is_error);
    assert!(result.content.contains("hello"));
}

#[tokio::test]
async fn test_exec_dangerous_command_blocked() {
    let (tool, _tmp) = test_exec(false);
    let result = tool.execute(r#"{"command": "rm -rf /"}"#).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_exec_missing_command_arg() {
    let (tool, _tmp) = test_exec(false);
    let result = tool.execute(r#"{}"#).await.unwrap();
    assert!(
        result.is_error,
        "expected error result, got: {}",
        result.content
    );
    assert!(
        result.content.contains("command"),
        "should mention missing 'command', got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_exec_timeout_kills_long_command() {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
    let tool = ExecTool::with_timeout(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(sandbox),
        Duration::from_secs(1),
    );

    let result = tool.execute(r#"{"command": "sleep 60"}"#).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("timed out"));
}

#[tokio::test]
async fn test_exec_inherits_quecto_env_vars() {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
    let tool = ExecTool::new(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox));

    let mut env_vars = HashMap::new();
    env_vars.insert("OPENAI_API_KEY".to_string(), "sk-secret".to_string());
    env_vars.insert("HOME".to_string(), "/home/testuser".to_string());

    let result = tool
        .execute_with_env(r#"{"command": "printenv OPENAI_API_KEY"}"#, &env_vars)
        .await
        .unwrap();
    assert!(result.content.contains("sk-secret"));
}

// --- Per-invocation timeout parameter ---

#[tokio::test]
async fn test_exec_per_invocation_timeout_kills_slow_command() {
    let (tool, _tmp) = test_exec(false);
    // Pass timeout=1 in JSON args — should kill sleep 10
    let result = tool
        .execute(r#"{"command": "sleep 10", "timeout": 1}"#)
        .await
        .unwrap();
    assert!(
        result.is_error,
        "slow command should be killed by per-invocation timeout"
    );
    assert!(
        result.content.contains("timed out"),
        "should mention timeout, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_exec_per_invocation_timeout_capped_at_max() {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
    // Configure tool with max 5s timeout
    let opts = ExecOptions {
        timeout: Duration::from_secs(5),
        ..ExecOptions::default()
    };
    let tool = ExecTool::with_options(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox), opts);
    // Request 99999s — should be capped at configured max (5s), command still runs
    let result = tool
        .execute(r#"{"command": "echo hi", "timeout": 99999}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "echo should succeed: {}", result.content);
    assert!(result.content.contains("hi"));
}

// --- commandPrefix option ---

#[tokio::test]
async fn test_exec_command_prefix_prepended() {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
    let opts = ExecOptions {
        command_prefix: Some("export MY_PREFIX_VAR=hello".to_string()),
        ..ExecOptions::default()
    };
    let tool = ExecTool::with_options(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox), opts);
    let result = tool
        .execute(r#"{"command": "echo $MY_PREFIX_VAR"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "echo should succeed: {}", result.content);
    assert!(
        result.content.contains("hello"),
        "prefix should set env: {}",
        result.content
    );
}

// --- Shell detection ---

#[tokio::test]
async fn test_exec_shell_detection_uses_shell_env() {
    let (tool, _tmp) = test_exec(false);
    // Set SHELL to /bin/sh and verify the shell is spawned (not an arbitrary binary).
    // $0 in the spawned shell prints the shell executable name.
    let mut env_overrides = HashMap::new();
    env_overrides.insert("SHELL".to_string(), "/bin/sh".to_string());
    let result = tool
        .execute_with_env(r#"{"command": "echo $0"}"#, &env_overrides)
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "shell detection should work: {}",
        result.content
    );
    assert!(
        result.content.contains("sh"),
        "output should name the shell, got: {}",
        result.content
    );
}

#[test]
fn test_exec_disallowed_shell_falls_back_to_sh() {
    // $SHELL pointing to a non-allowlisted binary should silently fall back to /bin/sh.
    // We test build_shell_command indirectly via the allowlist logic.
    // ALLOWED_SHELLS does not contain /tmp/evil, so it should use /bin/sh.
    // We can't call build_shell_command directly (it's private), but we can
    // confirm the constant list is correct.
    assert!(ALLOWED_SHELLS.contains(&"/bin/sh"));
    assert!(ALLOWED_SHELLS.contains(&"/bin/bash"));
    assert!(!ALLOWED_SHELLS.contains(&"/tmp/evil"));
}

// --- Truncation notice format ---

#[test]
fn test_exec_truncation_byte_notice_uses_truncate_tail() {
    use crate::infrastructure::tools::truncate::{TruncatedBy, truncate_tail};
    // Generate content that will be truncated by bytes (multi-line, >50KB)
    let line = "x".repeat(50) + "\n"; // 51 bytes per line
    let content: String = line.repeat(1500); // ~76.5KB
    let tr = truncate_tail(&content, 2000, 50 * 1024);
    assert!(tr.truncated, "should be truncated by bytes");
    assert_eq!(tr.truncated_by, Some(TruncatedBy::Bytes));
}

#[test]
fn test_exec_truncation_line_notice_uses_truncate_tail() {
    use crate::infrastructure::tools::truncate::{TruncatedBy, truncate_tail};
    // 3000 lines, each <50 bytes → truncated by lines
    let content: String = (1..=3000).map(|i| format!("line{}\n", i)).collect();
    let tr = truncate_tail(&content, 2000, 50 * 1024);
    assert!(tr.truncated, "should be truncated by lines");
    assert_eq!(tr.truncated_by, Some(TruncatedBy::Lines));
    assert!(
        tr.content.contains("line3000"),
        "tail should include the last line"
    );
}

#[tokio::test]
async fn test_exec_empty_object_returns_actionable_error() {
    let (tool, _tmp) = test_exec(false);
    let result = tool.execute("{}").await.unwrap();
    assert!(result.is_error, "expected error, got: {}", result.content);
    assert!(
        result.content.contains("command"),
        "should mention 'command', got: {}",
        result.content
    );
    assert!(
        result.content.contains("Example"),
        "should include example, got: {}",
        result.content
    );
}

#[test]
fn test_exec_description_includes_example() {
    let (tool, _tmp) = test_exec(false);
    let def = tool.definition();
    assert!(
        def.description.contains("Example"),
        "bash description should include Example, got: {}",
        def.description
    );
}

// --- parse_timeout tests ---

#[test]
fn test_parse_timeout_integer() {
    let args = serde_json::json!({"timeout": 30});
    assert_eq!(super::parse_timeout(&args), Some(Duration::from_secs(30)));
}

#[test]
fn test_parse_timeout_float() {
    let args = serde_json::json!({"timeout": 30.5});
    assert_eq!(super::parse_timeout(&args), Some(Duration::from_secs(31)));
}

#[test]
fn test_parse_timeout_zero() {
    let args = serde_json::json!({"timeout": 0});
    assert_eq!(super::parse_timeout(&args), None);
}

#[test]
fn test_parse_timeout_missing() {
    let args = serde_json::json!({"command": "ls"});
    assert_eq!(super::parse_timeout(&args), None);
}

#[test]
fn test_parse_timeout_negative() {
    let args = serde_json::json!({"timeout": -5});
    assert_eq!(super::parse_timeout(&args), None);
}

#[test]
fn test_parse_timeout_string_ignored() {
    let args = serde_json::json!({"timeout": "30"});
    assert_eq!(super::parse_timeout(&args), None);
}

// --- build_source_env tests ---

#[test]
fn test_build_source_env_with_overrides_inherits_all_vars() {
    let mut overrides = HashMap::new();
    overrides.insert("HOME".to_string(), "/tmp".to_string());
    overrides.insert("SECRET_VAR".to_string(), "yes".to_string());
    overrides.insert("QUECTO_SECRET_KEY".to_string(), "hunter2".to_string());
    let env = super::build_source_env(Some(&overrides));
    assert_eq!(env.get("HOME").map(|s| s.as_str()), Some("/tmp"));
    assert_eq!(env.get("SECRET_VAR").map(|s| s.as_str()), Some("yes"));
    assert_eq!(
        env.get("QUECTO_SECRET_KEY").map(|s| s.as_str()),
        Some("hunter2")
    );
}

#[test]
fn test_build_source_env_without_overrides() {
    let env = super::build_source_env(None);
    // Should contain at least HOME or PATH from the process env
    assert!(env.contains_key("HOME") || env.contains_key("PATH"));
}

// --- ALLOWED_SHELLS tests ---

#[test]
fn test_allowed_shells_contains_common() {
    assert!(super::ALLOWED_SHELLS.contains(&"/bin/sh"));
    assert!(super::ALLOWED_SHELLS.contains(&"/bin/bash"));
}

// --- build_shell_command (pure builder: program/args/env/cwd selection) ---

fn shell_program(env: &HashMap<String, String>) -> String {
    let ws = PathBuf::from("/tmp");
    let cmd = super::build_shell_command(&ws, "echo hi", env);
    cmd.as_std().get_program().to_string_lossy().into_owned()
}

#[test]
fn test_build_shell_command_uses_allowed_shell_from_env() {
    let mut env = HashMap::new();
    env.insert("SHELL".to_string(), "/bin/bash".to_string());
    assert_eq!(shell_program(&env), "/bin/bash");
}

#[test]
fn test_build_shell_command_rejects_disallowed_shell() {
    let mut env = HashMap::new();
    env.insert("SHELL".to_string(), "/tmp/evil".to_string());
    // Disallowed shell falls back to /bin/sh.
    assert_eq!(shell_program(&env), "/bin/sh");
}

#[test]
fn test_build_shell_command_defaults_to_sh_without_shell_env() {
    let env = HashMap::new();
    assert_eq!(shell_program(&env), "/bin/sh");
}

#[test]
fn test_build_shell_command_sets_args_and_cwd() {
    let ws = PathBuf::from("/tmp/some-workspace");
    let env = HashMap::new();
    let cmd = super::build_shell_command(&ws, "echo hello", &env);
    let std_cmd = cmd.as_std();
    let args: Vec<String> = std_cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    assert_eq!(args, vec!["-c".to_string(), "echo hello".to_string()]);
    assert_eq!(std_cmd.get_current_dir(), Some(ws.as_path()));
}

#[test]
fn test_build_shell_command_passes_through_source_env() {
    let mut env = HashMap::new();
    env.insert("MY_VAR".to_string(), "my_value".to_string());
    env.insert("PATH".to_string(), "/custom/bin".to_string());
    let cmd = super::build_shell_command(&PathBuf::from("/tmp"), "echo hi", &env);
    let std_cmd = cmd.as_std();
    let envs: HashMap<String, String> = std_cmd
        .get_envs()
        .filter_map(|(k, v)| {
            Some((
                k.to_string_lossy().into_owned(),
                v?.to_string_lossy().into_owned(),
            ))
        })
        .collect();
    assert_eq!(envs.get("MY_VAR").map(|s| s.as_str()), Some("my_value"));
    // Provided PATH is honored verbatim (no inherited-PATH injection).
    assert_eq!(envs.get("PATH").map(|s| s.as_str()), Some("/custom/bin"));
}

#[test]
fn test_build_shell_command_inherits_path_when_absent() {
    // When source env has no PATH, the process PATH is injected so the shell
    // can find binaries.
    let env = HashMap::new();
    let cmd = super::build_shell_command(&PathBuf::from("/tmp"), "echo hi", &env);
    let std_cmd = cmd.as_std();
    let has_path = std_cmd.get_envs().any(|(k, v)| k == "PATH" && v.is_some());
    // Only asserts when the test process itself has PATH (true in practice).
    if std::env::var("PATH").is_ok() {
        assert!(
            has_path,
            "PATH should be injected when absent from source env"
        );
    }
}

// --- make_exit_result (status -> ToolResult mapping) ---

#[cfg(unix)]
#[test]
fn test_make_exit_result_success() {
    use std::os::unix::process::ExitStatusExt;
    let status = std::process::ExitStatus::from_raw(0);
    let result = super::make_exit_result(status, "output text".to_string());
    assert!(!result.is_error);
    assert_eq!(result.content, "output text");
    assert!(result.image_blocks.is_empty());
}

#[cfg(unix)]
#[test]
fn test_make_exit_result_nonzero_exit_code() {
    use std::os::unix::process::ExitStatusExt;
    // raw status with low byte 0 and high byte 1 => exited with code 1.
    let status = std::process::ExitStatus::from_raw(1 << 8);
    let result = super::make_exit_result(status, "boom".to_string());
    assert!(result.is_error);
    assert!(
        result.content.starts_with("exit code 1\n"),
        "got: {}",
        result.content
    );
    assert!(result.content.contains("boom"));
}

#[cfg(unix)]
#[test]
fn test_make_exit_result_killed_by_signal_uses_minus_one() {
    use std::os::unix::process::ExitStatusExt;
    // Terminated by signal 9 => code() is None => falls back to -1.
    let status = std::process::ExitStatus::from_raw(9);
    let result = super::make_exit_result(status, "".to_string());
    assert!(result.is_error);
    assert!(
        result.content.contains("exit code -1"),
        "got: {}",
        result.content
    );
}

// --- read_stream_limited (in-memory AsyncRead, no real pipes) ---

#[tokio::test]
async fn test_read_stream_limited_under_cap() {
    let data = b"hello world".to_vec();
    let (content, truncated) = super::read_stream_limited(&data[..], 1024).await;
    assert_eq!(content, "hello world");
    assert!(!truncated);
}

#[tokio::test]
async fn test_read_stream_limited_truncates_over_cap() {
    let data = vec![b'x'; 10_000];
    let (content, truncated) = super::read_stream_limited(&data[..], 100).await;
    assert_eq!(content.len(), 100, "should keep only the cap");
    assert!(truncated, "exceeding the cap should set truncated=true");
}

#[tokio::test]
async fn test_read_stream_limited_empty_input() {
    let data: Vec<u8> = Vec::new();
    let (content, truncated) = super::read_stream_limited(&data[..], 100).await;
    assert!(content.is_empty());
    assert!(!truncated);
}

// --- await_stream_output / await_stream_output_with_timeout ---

#[tokio::test]
async fn test_await_stream_output_none() {
    let (s, t) = super::await_stream_output(None).await;
    assert!(s.is_empty());
    assert!(!t);
}

#[tokio::test]
async fn test_await_stream_output_some() {
    let handle = tokio::spawn(async { ("captured".to_string(), true) });
    let (s, t) = super::await_stream_output(Some(handle)).await;
    assert_eq!(s, "captured");
    assert!(t);
}

#[tokio::test]
async fn test_await_stream_output_with_timeout_none() {
    let (s, t) = super::await_stream_output_with_timeout(None, Duration::from_millis(50)).await;
    assert!(s.is_empty());
    assert!(!t);
}

#[tokio::test]
async fn test_await_stream_output_with_timeout_completes() {
    let handle = tokio::spawn(async { ("done".to_string(), false) });
    let (s, _t) =
        super::await_stream_output_with_timeout(Some(handle), Duration::from_secs(5)).await;
    assert_eq!(s, "done");
}

#[tokio::test]
async fn test_await_stream_output_with_timeout_times_out() {
    let handle = tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(30)).await;
        ("never".to_string(), false)
    });
    let (s, t) =
        super::await_stream_output_with_timeout(Some(handle), Duration::from_millis(20)).await;
    assert!(s.is_empty(), "timeout should yield empty output");
    assert!(!t);
}

// --- collect_and_truncate_output (combining + truncation + temp-file hint) ---

fn stream_tasks_from(
    stdout: Option<(String, bool)>,
    stderr: Option<(String, bool)>,
) -> super::StreamTasks {
    super::StreamTasks {
        stdout_task: stdout.map(|v| tokio::spawn(async move { v })),
        stderr_task: stderr.map(|v| tokio::spawn(async move { v })),
    }
}

#[tokio::test]
async fn test_collect_output_empty_when_no_tasks() {
    let mut tasks = stream_tasks_from(None, None);
    let out = super::collect_and_truncate_output(&mut tasks).await;
    assert!(out.is_empty());
}

#[tokio::test]
async fn test_collect_output_stdout_only_no_truncation() {
    let mut tasks = stream_tasks_from(Some(("just stdout".to_string(), false)), None);
    let out = super::collect_and_truncate_output(&mut tasks).await;
    assert_eq!(out, "just stdout");
}

#[tokio::test]
async fn test_collect_output_stderr_only_no_truncation() {
    let mut tasks = stream_tasks_from(None, Some(("just stderr".to_string(), false)));
    let out = super::collect_and_truncate_output(&mut tasks).await;
    assert_eq!(out, "just stderr");
}

#[tokio::test]
async fn test_collect_output_combines_stdout_and_stderr() {
    let mut tasks = stream_tasks_from(
        Some(("OUT".to_string(), false)),
        Some(("ERR".to_string(), false)),
    );
    let out = super::collect_and_truncate_output(&mut tasks).await;
    assert_eq!(out, "OUT\nERR");
}

#[tokio::test]
async fn test_collect_output_byte_truncation_includes_50kb_hint() {
    // Few lines but well over 50KB => byte truncation => "(50KB limit)" note.
    let big = "x".repeat(60 * 1024);
    let mut tasks = stream_tasks_from(Some((big, false)), None);
    let out = super::collect_and_truncate_output(&mut tasks).await;
    assert!(
        out.contains("Showing lines"),
        "got tail: {}",
        &out[out.len().saturating_sub(200)..]
    );
    assert!(out.contains("(50KB limit)"));
    assert!(out.contains("Full output"));
}

#[tokio::test]
async fn test_collect_output_line_truncation_hint_without_kb_note() {
    // Many short lines => line truncation => no "(50KB limit)" note.
    let many: String = (1..=3000).map(|i| format!("l{}\n", i)).collect();
    let mut tasks = stream_tasks_from(Some((many, false)), None);
    let out = super::collect_and_truncate_output(&mut tasks).await;
    assert!(out.contains("Showing lines"));
    assert!(
        !out.contains("(50KB limit)"),
        "line truncation must omit KB note"
    );
    assert!(out.contains("l3000"), "tail must keep the last line");
}

// --- run_command malformed-JSON arm (LLM-addressable error result) ---

#[tokio::test]
async fn test_exec_invalid_json_returns_error_result() {
    let (tool, _tmp) = test_exec(false);
    let result = tool.execute("{ not valid json").await.unwrap();
    assert!(
        result.is_error,
        "malformed JSON should yield is_error result"
    );
    assert!(
        result.content.contains("invalid JSON arguments"),
        "got: {}",
        result.content
    );
    assert!(result.content.contains("Example"));
}

// --- ExecTool accessors / Debug / ExecOptions ---

#[test]
fn test_exec_tool_timeout_getter_and_debug() {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
    let tool = ExecTool::with_timeout(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(sandbox),
        Duration::from_secs(42),
    );
    assert_eq!(tool.timeout(), Duration::from_secs(42));
    let dbg = format!("{:?}", tool);
    assert!(dbg.contains("ExecTool"));
    assert!(dbg.contains("42") || dbg.contains("max_capture_bytes"));
}

#[test]
fn test_exec_options_default_and_clone() {
    let opts = ExecOptions::default();
    assert_eq!(opts.timeout, Duration::MAX);
    assert!(opts.command_prefix.is_none());
    let cloned = opts.clone();
    assert_eq!(cloned.max_capture_bytes, opts.max_capture_bytes);
    // Debug derive exercised.
    assert!(format!("{:?}", cloned).contains("ExecOptions"));
}
