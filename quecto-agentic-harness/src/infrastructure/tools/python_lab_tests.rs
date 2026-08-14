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
    assert!(cancelled.content.contains("cancelled"));
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
    assert_eq!(v["stdout_truncated"], true);
    assert_eq!(v["stderr_truncated"], true);
    let artifacts = v["artifact_paths"].as_array().unwrap();
    assert_eq!(artifacts.len(), 2);
    assert!(
        artifacts
            .iter()
            .any(|p| p.as_str().unwrap().ends_with("stdout.txt"))
    );
    assert!(
        artifacts
            .iter()
            .any(|p| p.as_str().unwrap().ends_with("stderr.txt"))
    );
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
async fn background_completion_retains_result_and_output_pages() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = tool(tmp.path());
    let started = lab.execute(r#"{"op":"run","code":"import sys; print('abcdef'); sys.stderr.write('uvwxyz')","background":true,"max_output_bytes":4}"#).await.unwrap();
    let v: serde_json::Value = serde_json::from_str(&started.content).unwrap();
    let job_id = v["job_id"].as_str().unwrap();
    for _ in 0..20 {
        let status = lab
            .execute(&format!(r#"{{"op":"status","job_id":"{}"}}"#, job_id))
            .await
            .unwrap();
        if status.content.contains("completed") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let status = lab
        .execute(&format!(r#"{{"op":"status","job_id":"{}"}}"#, job_id))
        .await
        .unwrap();
    assert!(status.content.contains("completed"), "{}", status.content);
    let page = lab
        .execute(&format!(
            r#"{{"op":"output","job_id":"{}","offset":2,"limit":3}}"#,
            job_id
        ))
        .await
        .unwrap();
    let out: serde_json::Value = serde_json::from_str(&page.content).unwrap();
    assert_eq!(out["stdout"], "cde");
    assert_eq!(out["stdout_more"], true);
    assert!(out["result"]["output_truncated"].as_bool().unwrap());
}

#[tokio::test]
async fn concurrent_background_jobs_are_capped_until_cancelled() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = PythonLabTool::new(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()), true)),
        PythonLabConfig {
            max_concurrent_jobs: 1,
            default_timeout_seconds: 5,
            ..Default::default()
        },
    );
    let first = lab
        .execute(r#"{"op":"run","code":"import time; time.sleep(5)","background":true}"#)
        .await
        .unwrap();
    let first_v: serde_json::Value = serde_json::from_str(&first.content).unwrap();
    let second = lab
        .execute(r#"{"op":"run","code":"print(1)","background":true}"#)
        .await
        .unwrap();
    assert!(second.is_error);
    assert!(second.content.contains("concurrent job limit"));
    let job_id = first_v["job_id"].as_str().unwrap();
    lab.execute(&format!(r#"{{"op":"cancel","job_id":"{}"}}"#, job_id))
        .await
        .unwrap();
}

#[tokio::test]
async fn default_environment_filters_sensitive_variables() {
    let tmp = tempfile::tempdir().unwrap();
    // SAFETY: this test mutates a unique environment key and removes it before returning.
    unsafe { std::env::set_var("PYTHON_LAB_SECRET_SHOULD_NOT_LEAK", "secret-value") };
    let result = tool(tmp.path())
        .execute(r#"{"op":"run","code":"import os; print(os.environ.get('PYTHON_LAB_SECRET_SHOULD_NOT_LEAK', 'missing'))"}"#)
        .await
        .unwrap();
    // SAFETY: paired cleanup for the unique key set above.
    unsafe { std::env::remove_var("PYTHON_LAB_SECRET_SHOULD_NOT_LEAK") };
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["stdout"], "missing\n");
}

#[tokio::test]
async fn absolute_and_symlink_script_escapes_are_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_script = outside.path().join("outside.py");
    std::fs::write(&outside_script, "print('outside')").unwrap();
    let abs = tool(tmp.path())
        .execute(&format!(
            r#"{{"op":"run","path":"{}"}}"#,
            outside_script.display()
        ))
        .await
        .unwrap();
    assert!(abs.is_error);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside_script, tmp.path().join("link.py")).unwrap();
        let sym = tool(tmp.path())
            .execute(r#"{"op":"run","path":"link.py"}"#)
            .await
            .unwrap();
        assert!(sym.is_error, "{}", sym.content);
    }
}

#[tokio::test]
async fn repeated_truncated_runs_use_distinct_artifacts() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = tool(tmp.path());
    let a = lab
        .execute(r#"{"op":"run","code":"print('aaaaaaaaaaaaaaaa')","max_output_bytes":4}"#)
        .await
        .unwrap();
    let b = lab
        .execute(r#"{"op":"run","code":"print('bbbbbbbbbbbbbbbb')","max_output_bytes":4}"#)
        .await
        .unwrap();
    let av: serde_json::Value = serde_json::from_str(&a.content).unwrap();
    let bv: serde_json::Value = serde_json::from_str(&b.content).unwrap();
    assert_ne!(av["artifact_paths"][0], bv["artifact_paths"][0]);
}

#[tokio::test]
async fn background_output_is_empty_before_artifacts_exist() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = tool(tmp.path());
    let started = lab
        .execute(r#"{"op":"run","code":"import time; time.sleep(1)","background":true}"#)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&started.content).unwrap();
    let job_id = v["job_id"].as_str().unwrap();
    for entry in std::fs::read_dir(tmp.path().join(".quecto/python_lab")).unwrap() {
        let dir = entry.unwrap().path();
        let _ = std::fs::remove_file(dir.join("stdout.txt"));
        let _ = std::fs::remove_file(dir.join("stderr.txt"));
    }
    let output = lab
        .execute(&format!(r#"{{"op":"output","job_id":"{}"}}"#, job_id))
        .await
        .unwrap();
    assert!(!output.is_error, "{}", output.content);
    let out: serde_json::Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(out["stdout"], "");
    assert_eq!(out["stderr"], "");
    let _ = lab
        .execute(&format!(r#"{{"op":"cancel","job_id":"{}"}}"#, job_id))
        .await;
}

#[tokio::test]
async fn cancelling_completed_background_job_preserves_terminal_status() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = tool(tmp.path());
    let started = lab
        .execute(r#"{"op":"run","code":"print('done')","background":true}"#)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&started.content).unwrap();
    let job_id = v["job_id"].as_str().unwrap();
    for _ in 0..20 {
        let status = lab
            .execute(&format!(r#"{{"op":"status","job_id":"{}"}}"#, job_id))
            .await
            .unwrap();
        if status.content.contains("completed") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let cancel = lab
        .execute(&format!(r#"{{"op":"cancel","job_id":"{}"}}"#, job_id))
        .await
        .unwrap();
    assert!(!cancel.is_error, "{}", cancel.content);
    assert!(cancel.content.contains("completed"), "{}", cancel.content);
}
