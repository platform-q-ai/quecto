use std::sync::Arc;

use crate::domain::tool::Tool;
use crate::infrastructure::security::sandbox::Sandbox;

use super::python_lab::{PythonLabConfig, PythonLabTool};

fn tool(dir: &std::path::Path) -> PythonLabTool {
    PythonLabTool::new(
        Arc::new(dir.to_path_buf()),
        Arc::new(Sandbox::new(Some(dir.to_path_buf()), true)),
        PythonLabConfig {
            default_timeout_seconds: 1,
            max_foreground_seconds: 2,
            default_max_output_bytes: 8,
            max_output_bytes: 32,
            ..Default::default()
        },
    )
}

#[tokio::test]
async fn inline_executes_and_reports_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let result = tool(tmp.path())
        .execute(r#"{"op":"run","code":"print(sum(range(5)))"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["status"], "completed");
    assert_eq!(v["stdout"], "10\n");
    assert_eq!(v["invocation_type"], "inline");
    assert!(v["execution_id"].as_str().unwrap().starts_with("py_"));
}

#[tokio::test]
async fn file_executes_with_args_stdin_and_persists_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a file.py"), "import sys, pathlib\npathlib.Path('out.txt').write_text(sys.stdin.read()+sys.argv[1])\nprint(sys.argv[1])").unwrap();
    let result = tool(tmp.path())
        .execute(r#"{"op":"run","path":"a file.py","args":[";$HOME"],"stdin":"in:"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("out.txt")).unwrap(),
        "in:;$HOME"
    );
}

#[tokio::test]
async fn rejects_code_path_cardinality_and_path_escape() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(
        tool(tmp.path())
            .execute(r#"{"op":"run","code":"x=1","path":"x.py"}"#)
            .await
            .unwrap()
            .is_error
    );
    assert!(
        tool(tmp.path())
            .execute(r#"{"op":"run"}"#)
            .await
            .unwrap()
            .is_error
    );
    let escaped = tool(tmp.path())
        .execute(r#"{"op":"run","path":"../x.py"}"#)
        .await;
    assert!(escaped.is_err());
}

#[tokio::test]
async fn truncates_output_to_workspace_relative_artifact() {
    let tmp = tempfile::tempdir().unwrap();
    let result = tool(tmp.path())
        .execute(r#"{"op":"run","code":"print('abcdefghijklmnop')","max_output_bytes":4}"#)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["output_truncated"], true);
    assert_eq!(v["stdout"], "abcd");
    let artifact = v["artifact_paths"][0].as_str().unwrap();
    assert!(!artifact.starts_with('/'));
    assert!(
        std::fs::read_to_string(tmp.path().join(artifact))
            .unwrap()
            .contains("abcdefghijklmnop")
    );
}

#[tokio::test]
async fn timeout_reports_timed_out() {
    let tmp = tempfile::tempdir().unwrap();
    let result = tool(tmp.path())
        .execute(r#"{"op":"run","code":"while True: pass","timeout_seconds":1}"#)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["status"], "timed_out");
    assert!(result.is_error);
}

#[tokio::test]
async fn file_named_dash_c_runs_as_script_not_interpreter_option() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("-c"), "print('from-file')").unwrap();
    let result = tool(tmp.path())
        .execute(r#"{"op":"run","path":"-c"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["stdout"], "from-fil");
}

#[tokio::test]
async fn rejects_malformed_args_and_limit_types() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(
        tool(tmp.path())
            .execute(r#"{"op":"run","code":"print(1)","args":[1]}"#)
            .await
            .unwrap()
            .is_error
    );
    assert!(
        tool(tmp.path())
            .execute(r#"{"op":"run","code":"print(1)","timeout_seconds":-1}"#)
            .await
            .unwrap()
            .is_error
    );
    assert!(
        tool(tmp.path())
            .execute(r#"{"op":"run","code":"print(1)","max_output_bytes":1.5}"#)
            .await
            .unwrap()
            .is_error
    );
}

#[tokio::test]
async fn deferred_operations_report_not_implemented() {
    let tmp = tempfile::tempdir().unwrap();
    for args in [
        r#"{"op":"status","job_id":"j"}"#,
        r#"{"op":"output","job_id":"j"}"#,
        r#"{"op":"cancel","job_id":"j"}"#,
        r#"{"op":"run","code":"print(1)","background":true}"#,
    ] {
        let result = tool(tmp.path()).execute(args).await.unwrap();
        assert!(result.is_error);
        let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(v["status"], "not_implemented");
    }
}

#[tokio::test]
async fn nonzero_exit_reports_exit_code_and_error() {
    let tmp = tempfile::tempdir().unwrap();
    let result = tool(tmp.path())
        .execute(r#"{"op":"run","code":"import sys; sys.exit(7)"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["status"], "completed");
    assert_eq!(v["exit_code"], 7);
}

#[tokio::test]
async fn stderr_truncation_uses_stderr_artifact() {
    let tmp = tempfile::tempdir().unwrap();
    let result = tool(tmp.path())
        .execute(r#"{"op":"run","code":"import sys; sys.stderr.write('abcdefghijklmnop')","max_output_bytes":4}"#)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["stderr"], "abcd");
    assert_eq!(v["stderr_truncated"], true);
    let artifact = v["artifact_paths"][0].as_str().unwrap();
    assert!(artifact.ends_with("stderr.txt"));
    assert!(
        std::fs::read_to_string(tmp.path().join(artifact))
            .unwrap()
            .contains("abcdefghijklmnop")
    );
}

#[test]
fn python_lab_config_defaults_and_conversion_are_stable() {
    let tool_cfg = super::python_lab::PythonLabToolConfig::default();
    assert_eq!(tool_cfg.default_timeout_seconds, 60);
    assert_eq!(tool_cfg.max_foreground_seconds, 300);
    assert_eq!(tool_cfg.max_background_seconds, 1800);
    assert_eq!(tool_cfg.default_max_output_bytes, 200_000);
    assert_eq!(tool_cfg.max_output_bytes, 1_000_000);
    assert_eq!(tool_cfg.max_processes, Some(1));
    assert_eq!(tool_cfg.max_concurrent_jobs, 2);
    assert!(!tool_cfg.inherit_environment);
    let runtime_cfg = PythonLabConfig::from(tool_cfg);
    assert_eq!(runtime_cfg.default_timeout_seconds, 60);
    assert_eq!(runtime_cfg.max_output_bytes, 1_000_000);
}
