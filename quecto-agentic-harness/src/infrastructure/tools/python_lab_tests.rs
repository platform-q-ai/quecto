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
        .await
        .unwrap();
    assert!(escaped.is_error);
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
    // The artifact keeps the full output even though the inline preview was
    // capped at max_output_bytes, so truncated output stays recoverable.
    assert_eq!(
        std::fs::read_to_string(tmp.path().join(artifact)).unwrap(),
        "abcdefghijklmnop\n"
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
async fn background_operations_start_report_output_and_cancel() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = tool(tmp.path());
    let started = lab
        .execute(r#"{"op":"run","code":"import time; print('ready', flush=True); time.sleep(5)","background":true}"#)
        .await
        .unwrap();
    assert!(!started.is_error, "{}", started.content);
    let v: serde_json::Value = serde_json::from_str(&started.content).unwrap();
    assert_eq!(v["status"], "running");
    let job_id = v["job_id"].as_str().unwrap();
    let status = lab
        .execute(&format!(r#"{{"op":"status","job_id":"{}"}}"#, job_id))
        .await
        .unwrap();
    assert!(status.content.contains("running"));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let output = lab
        .execute(&format!(
            r#"{{"op":"output","job_id":"{}","limit":5}}"#,
            job_id
        ))
        .await
        .unwrap();
    assert!(output.content.contains("stdout"));
    let cancelled = lab
        .execute(&format!(r#"{{"op":"cancel","job_id":"{}"}}"#, job_id))
        .await
        .unwrap();
    assert!(!cancelled.is_error, "{}", cancelled.content);
    assert!(cancelled.content.contains("cancelling"));
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
    assert_eq!(
        std::fs::read_to_string(tmp.path().join(artifact)).unwrap(),
        "abcdefghijklmnop"
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

#[tokio::test]
async fn invalid_json_and_non_array_args_are_tool_errors() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(tool(tmp.path()).execute("not-json").await.unwrap().is_error);
    assert!(
        tool(tmp.path())
            .execute(r#"{"op":"run","code":"print(1)","args":"bad"}"#)
            .await
            .unwrap()
            .is_error
    );
}

#[tokio::test]
async fn session_key_and_limit_clamping_are_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = tool(tmp.path());
    lab.set_session_key("session-a".to_string());
    let result = lab
        .execute(
            r#"{"op":"run","code":"print('ok')","timeout_seconds":999,"max_output_bytes":999}"#,
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["session_id"], "session-a");
    assert_eq!(v["timeout_seconds"], 2);
    assert_eq!(v["stdout"], "ok\n");
    assert_eq!(v["output_truncated"], false);
    assert!(v["artifact_paths"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn inherit_environment_can_be_enabled() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = PythonLabTool::new(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()), true)),
        PythonLabConfig {
            inherit_environment: true,
            default_max_output_bytes: 32,
            ..Default::default()
        },
    );
    let result = lab
        .execute(r#"{"op":"run","code":"import os; print('PATH' in os.environ)"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["stdout"], "True\n");
}

#[test]
fn inherited_child_policy_snapshot_trait_defaults_are_covered() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = tool(tmp.path());
    lab.set_inherited_child_policy_snapshot_for_spawn(std::collections::BTreeMap::new());
    assert!(lab.inherited_child_policy_snapshot_for_spawn().is_none());
}

#[tokio::test]
async fn default_op_and_file_change_reporting_cover_metadata_edges() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("nested")).unwrap();
    let result = tool(tmp.path())
        .execute(
            r#"{"code":"from pathlib import Path\nPath('created.txt').write_text('new')\nPath('nested/changed.txt').write_text('changed')"}"#,
        )
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["status"], "completed");
    let changed = v["files_created_or_modified"].as_array().unwrap();
    assert!(changed.iter().any(|p| p == "created.txt"));
    assert!(changed.iter().any(|p| p == "nested/changed.txt"));
}

#[tokio::test]
async fn stderr_and_stdout_truncation_report_both_artifacts() {
    let tmp = tempfile::tempdir().unwrap();
    let result = tool(tmp.path())
        .execute(
            r#"{"op":"run","code":"import sys; print('abcdefghijklmnop'); sys.stderr.write('qrstuvwxyz')","max_output_bytes":4}"#,
        )
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["output_truncated"], true);
    assert!(v["stdout_truncated"].as_bool().unwrap() || v["stderr_truncated"].as_bool().unwrap());
    let artifacts = v["artifact_paths"].as_array().unwrap();
    assert!(!artifacts.is_empty());
}

#[test]
fn python_lab_config_deserializes_partial_and_full_json_shapes() {
    let partial: super::python_lab::PythonLabToolConfig =
        serde_json::from_value(serde_json::json!({
            "max_output_bytes": 1234,
            "inherit_environment": true
        }))
        .unwrap();
    assert_eq!(partial.default_timeout_seconds, 60);
    assert_eq!(partial.max_output_bytes, 1234);
    assert!(partial.inherit_environment);

    let full: super::python_lab::PythonLabToolConfig = serde_json::from_value(serde_json::json!({
        "default_timeout_seconds": 5,
        "max_foreground_seconds": 6,
        "max_background_seconds": 7,
        "default_max_output_bytes": 8,
        "max_output_bytes": 9,
        "max_memory_bytes": 10,
        "max_cpu_seconds": 11,
        "max_processes": null,
        "max_concurrent_jobs": 12,
        "inherit_environment": false
    }))
    .unwrap();
    assert_eq!(full.default_timeout_seconds, 5);
    assert_eq!(full.max_foreground_seconds, 6);
    assert_eq!(full.max_background_seconds, 7);
    assert_eq!(full.default_max_output_bytes, 8);
    assert_eq!(full.max_output_bytes, 9);
    assert_eq!(full.max_memory_bytes, Some(10));
    assert_eq!(full.max_cpu_seconds, Some(11));
    assert_eq!(full.max_processes, None);
    assert_eq!(full.max_concurrent_jobs, 12);
}

#[tokio::test]
async fn capped_stdout_is_drained_so_program_can_finish() {
    let tmp = tempfile::tempdir().unwrap();
    let code = "import pathlib, sys\nsys.stdout.write('x' * 200000)\nsys.stdout.flush()\npathlib.Path('finished.txt').write_text('ok')";
    let result = tool(tmp.path())
        .execute(&serde_json::json!({"op":"run","code":code,"max_output_bytes":4}).to_string())
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("finished.txt")).unwrap(),
        "ok"
    );
}

#[tokio::test]
async fn max_output_bytes_caps_each_stream_artifact_independently() {
    let tmp = tempfile::tempdir().unwrap();
    let result = tool(tmp.path())
        .execute(r#"{"op":"run","code":"import sys; sys.stdout.write('a' * 40); sys.stdout.flush(); sys.stderr.write('w' * 40); sys.stderr.flush()","max_output_bytes":4}"#)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    let exec_id = v["execution_id"].as_str().unwrap();
    let stdout_artifact = format!(".quecto/python_lab/{exec_id}/stdout.txt");
    let stderr_artifact = format!(".quecto/python_lab/{exec_id}/stderr.txt");
    // Each stream gets its own budget sized by the configured hard cap, so a
    // flooding stdout cannot consume the allowance stderr needs for a traceback.
    for artifact in [stdout_artifact, stderr_artifact] {
        let len = std::fs::metadata(tmp.path().join(&artifact)).unwrap().len();
        assert_eq!(
            len, 32,
            "{artifact} persisted {len} bytes: {}",
            result.content
        );
    }
}

#[tokio::test]
async fn exact_cap_stdout_is_not_reported_truncated() {
    let tmp = tempfile::tempdir().unwrap();
    let result = tool(tmp.path())
        .execute(
            r#"{"op":"run","code":"import sys; sys.stdout.write('abcd')","max_output_bytes":4}"#,
        )
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["stdout"], "abcd");
    assert_eq!(v["output_truncated"], false);
    assert_eq!(v["stdout_truncated"], false);
    assert!(v["artifact_paths"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn exact_cap_stderr_is_not_reported_truncated() {
    let tmp = tempfile::tempdir().unwrap();
    let result = tool(tmp.path())
        .execute(
            r#"{"op":"run","code":"import sys; sys.stderr.write('wxyz')","max_output_bytes":4}"#,
        )
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["stderr"], "wxyz");
    assert_eq!(v["output_truncated"], false);
    assert_eq!(v["stderr_truncated"], false);
    assert!(v["artifact_paths"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn exact_combined_stdout_stderr_cap_is_not_reported_truncated() {
    let tmp = tempfile::tempdir().unwrap();
    let result = tool(tmp.path())
        .execute(r#"{"op":"run","code":"import sys; sys.stdout.write('ab'); sys.stdout.flush(); sys.stderr.write('cd'); sys.stderr.flush()","max_output_bytes":4}"#)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["output_truncated"], false);
    assert_eq!(v["stdout_truncated"], false);
    assert_eq!(v["stderr_truncated"], false);
    assert!(v["artifact_paths"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn deeply_nested_workspace_does_not_overflow_the_snapshot_walk() {
    // A recursive walk here overflows the worker thread's stack, which aborts
    // the process rather than unwinding, so a program could kill the harness.
    let tmp = tempfile::tempdir().unwrap();
    let result = tool(tmp.path())
        .execute(
            r#"{"op":"run","code":"import os\nfor _ in range(400):\n    os.mkdir('d'); os.chdir('d')\n","timeout_seconds":2}"#,
        )
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["status"], "completed", "{}", result.content);
    // The follow-up run also walks the tree the first one left behind, so this
    // proves the deep directories do not poison every subsequent execution.
    let again = tool(tmp.path())
        .execute(r#"{"op":"run","code":"print('ok')"}"#)
        .await
        .unwrap();
    let again_v: serde_json::Value = serde_json::from_str(&again.content).unwrap();
    assert_eq!(again_v["status"], "completed", "{}", again.content);
    assert_eq!(again_v["stdout"], "ok\n");
}

#[tokio::test]
async fn own_artifacts_are_not_reported_as_program_writes() {
    let tmp = tempfile::tempdir().unwrap();
    let result = tool(tmp.path())
        .execute(r#"{"op":"run","code":"open('made.txt','w').write('x')"}"#)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    let changed: Vec<&str> = v["files_created_or_modified"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    assert_eq!(changed, vec!["made.txt"], "{}", result.content);
}

#[tokio::test]
async fn script_paths_inside_the_artifact_directory_are_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let result = tool(tmp.path())
        .execute(r#"{"op":"run","path":".quecto/python_lab/planted.py"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.content.contains("security violation"),
        "{}",
        result.content
    );
}

#[tokio::test]
async fn preview_larger_than_one_read_is_returned_whole() {
    // tokio caps a single read at 2 MiB; a lone read() would silently return a
    // short preview while reporting output_truncated: false.
    let tmp = tempfile::tempdir().unwrap();
    let lab = PythonLabTool::new(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()), true)),
        PythonLabConfig {
            default_timeout_seconds: 30,
            default_max_output_bytes: 8_000_000,
            max_output_bytes: 8_000_000,
            ..Default::default()
        },
    );
    let result = lab
        .execute(r#"{"op":"run","code":"import sys; sys.stdout.write('x' * 3000000)"}"#)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["stdout"].as_str().unwrap().len(), 3_000_000);
    assert_eq!(v["output_truncated"], false);
}

#[tokio::test]
async fn snapshot_depth_cap_bounds_reported_changes() {
    // Without an assertion on the cap itself, the iterative rewrite alone
    // satisfies the deep-tree test and removing the cap would go unnoticed.
    let tmp = tempfile::tempdir().unwrap();
    let result = tool(tmp.path())
        .execute(
            r#"{"op":"run","code":"import os\nfor _ in range(80):\n    os.mkdir('d'); os.chdir('d')\nopen('deep.txt','w').write('x')\n","timeout_seconds":2}"#,
        )
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["status"], "completed", "{}", result.content);
    let changed: Vec<&str> = v["files_created_or_modified"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|x| x.as_str())
        .collect();
    assert!(
        !changed.iter().any(|p| p.ends_with("deep.txt")),
        "a file below the depth cap should not be walked: {changed:?}"
    );
}

#[tokio::test]
async fn wide_directory_snapshot_is_bounded() {
    // The entry cap was only checked per directory, so one flat directory with
    // more files than the cap was walked in full.
    let tmp = tempfile::tempdir().unwrap();
    let result = tool(tmp.path())
        .execute(
            r#"{"op":"run","code":"import os\nos.mkdir('many')\nfor i in range(25000):\n    open(f'many/f{i}','w').close()\n","timeout_seconds":20}"#,
        )
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["status"], "completed", "{}", result.content);
    let changed = v["files_created_or_modified"].as_array().unwrap().len();
    assert!(
        changed <= 20_000,
        "snapshot should be bounded by the entry cap, reported {changed}"
    );
}
