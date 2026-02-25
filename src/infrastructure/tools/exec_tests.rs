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
    let result = tool.execute(r#"{}"#).await;
    assert!(result.is_err());
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
