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
