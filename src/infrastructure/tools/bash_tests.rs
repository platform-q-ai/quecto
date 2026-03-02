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
async fn test_exec_strips_quecto_env_vars() {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
    let tool = ExecTool::new(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox));

    let mut env_vars = HashMap::new();
    env_vars.insert(
        "QUECTO_PROVIDERS_OPENAI_API_KEY".to_string(),
        "sk-secret".to_string(),
    );
    env_vars.insert("HOME".to_string(), "/home/testuser".to_string());

    let result = tool
        .execute_with_env(
            r#"{"command": "printenv QUECTO_PROVIDERS_OPENAI_API_KEY"}"#,
            &env_vars,
        )
        .await
        .unwrap();
    assert!(!result.content.contains("sk-secret"));
}

#[test]
fn test_nsjail_falls_back_when_binary_missing() {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
    let opts = ExecOptions {
        isolation_mode: ExecIsolationMode::Nsjail,
        allow_native_fallback: true,
        nsjail: NsjailOptions {
            binary: "definitely-not-a-real-binary".to_string(),
            ..NsjailOptions::default()
        },
        ..ExecOptions::default()
    };
    let tool = ExecTool::with_options(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox), opts);
    assert_eq!(tool.mode(), ExecIsolationMode::Native);
    assert!(
        tool.startup_warning()
            .unwrap_or_default()
            .contains("falling back to native")
    );
    assert!(tool.startup_error().is_none());
}

#[tokio::test]
async fn test_nsjail_missing_without_fallback_returns_config_error() {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
    let opts = ExecOptions {
        isolation_mode: ExecIsolationMode::Nsjail,
        nsjail: NsjailOptions {
            binary: "definitely-not-a-real-binary".to_string(),
            ..NsjailOptions::default()
        },
        ..ExecOptions::default()
    };
    let tool = ExecTool::with_options(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox), opts);

    let result = tool.execute(r#"{"command":"echo hi"}"#).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("allow_native_fallback")
    );
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
