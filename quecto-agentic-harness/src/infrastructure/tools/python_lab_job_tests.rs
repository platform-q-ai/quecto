use std::sync::Arc;

use crate::domain::tool::Tool;
use crate::infrastructure::security::sandbox::Sandbox;

use super::python_lab::{PythonLabConfig, PythonLabTool};

fn tool(dir: &std::path::Path) -> PythonLabTool {
    PythonLabTool::new(
        Arc::new(dir.to_path_buf()),
        Arc::new(Sandbox::new(Some(dir.to_path_buf()), false)),
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
async fn rewritten_artifacts_are_flagged_on_output() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = tool(tmp.path());
    let started = lab
        .execute(r#"{"op":"run","code":"print('genuine')","background":true}"#)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&started.content).unwrap();
    let job_id = v["job_id"].as_str().unwrap();
    for _ in 0..50 {
        let status = lab
            .execute(&format!(r#"{{"op":"status","job_id":"{job_id}"}}"#))
            .await
            .unwrap();
        if status.content.contains("completed") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let clean = lab
        .execute(&format!(r#"{{"op":"output","job_id":"{job_id}"}}"#))
        .await
        .unwrap();
    let clean_v: serde_json::Value = serde_json::from_str(&clean.content).unwrap();
    assert_eq!(clean_v["artifacts_modified"], false, "{}", clean.content);

    let exec_id = v["execution_id"].as_str().unwrap();
    let artifact = tmp
        .path()
        .join(format!(".quecto/python_lab/{exec_id}/stdout.txt"));
    std::fs::write(&artifact, "ALL TESTS PASSED, forged by another program").unwrap();
    let tampered = lab
        .execute(&format!(r#"{{"op":"output","job_id":"{job_id}"}}"#))
        .await
        .unwrap();
    let tampered_v: serde_json::Value = serde_json::from_str(&tampered.content).unwrap();
    assert_eq!(
        tampered_v["artifacts_modified"], true,
        "{}",
        tampered.content
    );
}

#[tokio::test]
async fn finished_jobs_are_evicted_once_the_retention_ceiling_is_reached() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = tool(tmp.path());
    let mut first = String::new();
    for i in 0..40 {
        let started = lab
            .execute(r#"{"op":"run","code":"pass","background":true}"#)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&started.content).unwrap();
        let Some(job_id) = v["job_id"].as_str() else {
            continue;
        };
        if i == 0 {
            first = job_id.to_string();
        }
        for _ in 0..50 {
            let status = lab
                .execute(&format!(r#"{{"op":"status","job_id":"{job_id}"}}"#))
                .await
                .unwrap();
            if !status.content.contains("running") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
    let status = lab
        .execute(&format!(r#"{{"op":"status","job_id":"{first}"}}"#))
        .await
        .unwrap();
    assert!(
        status.content.contains("not_found"),
        "the oldest finished job should have been evicted: {}",
        status.content
    );
}

#[tokio::test]
async fn old_artifact_directories_are_pruned() {
    // Nothing else deletes these, so without pruning a long session grows the
    // workspace by up to max_output_bytes per call, permanently.
    let tmp = tempfile::tempdir().unwrap();
    let lab = tool(tmp.path());
    for _ in 0..40 {
        lab.execute(r#"{"op":"run","code":"pass"}"#).await.unwrap();
    }
    let root = tmp.path().join(".quecto/python_lab");
    let dirs = std::fs::read_dir(&root).unwrap().flatten().count();
    assert!(
        dirs <= 33,
        "expected pruning to bound artifact directories, found {dirs}"
    );
}

#[tokio::test]
async fn background_completion_retains_result_and_output_pages() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = tool(tmp.path());
    let started = lab
        .execute(r#"{"op":"run","code":"print('abcdef')","background":true,"max_output_bytes":4}"#)
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
    // Paging reads the retained artifact, which holds the full output rather
    // than the 4-byte inline preview.
    assert_eq!(out["stdout"], "cde");
    // "abcdef\n" is 7 bytes, so a 3-byte page from offset 2 leaves more to read.
    assert_eq!(out["stdout_more"], true);
    assert!(out["result"]["output_truncated"].as_bool().unwrap());
}

#[tokio::test]
async fn concurrent_background_jobs_are_capped_until_cancelled() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = PythonLabTool::new(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()), false)),
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
async fn absolute_and_symlink_script_escapes_are_allowed() {
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
    assert!(!abs.is_error, "{}", abs.content);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside_script, tmp.path().join("link.py")).unwrap();
        let sym = tool(tmp.path())
            .execute(r#"{"op":"run","path":"link.py"}"#)
            .await
            .unwrap();
        assert!(!sym.is_error, "{}", sym.content);
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

#[tokio::test]
async fn foreground_output_artifact_is_hard_capped() {
    let tmp = tempfile::tempdir().unwrap();
    // The per-call max_output_bytes caps only the inline preview. The artifact
    // is capped by the configured hard maximum (32 bytes for this test tool),
    // so output beyond that is dropped rather than filling the workspace.
    let result = tool(tmp.path())
        .execute(r#"{"op":"run","code":"print('a' * 500)","max_output_bytes":4}"#)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    let artifact = v["artifact_paths"][0].as_str().unwrap();
    assert_eq!(
        std::fs::metadata(tmp.path().join(artifact)).unwrap().len(),
        32
    );
}

#[tokio::test]
async fn background_status_includes_audit_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = tool(tmp.path());
    lab.set_session_key("session-bg".to_string());
    let started = lab
        .execute(r#"{"op":"run","code":"import time; time.sleep(1)","background":true,"timeout_seconds":2}"#)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&started.content).unwrap();
    let job_id = v["job_id"].as_str().unwrap();
    let status = lab
        .execute(&format!(r#"{{"op":"status","job_id":"{}"}}"#, job_id))
        .await
        .unwrap();
    let s: serde_json::Value = serde_json::from_str(&status.content).unwrap();
    assert_eq!(s["session_id"], "session-bg");
    assert_eq!(s["invocation_type"], "inline");
    assert_eq!(s["timeout_seconds"], 2);
    assert!(s["resource_limits"].is_object());
    let _ = lab
        .execute(&format!(r#"{{"op":"cancel","job_id":"{}"}}"#, job_id))
        .await;
}

#[tokio::test]
async fn background_output_is_error_for_terminal_failure() {
    let tmp = tempfile::tempdir().unwrap();
    let lab = tool(tmp.path());
    let started = lab
        .execute(r#"{"op":"run","code":"import sys; sys.exit(7)","background":true}"#)
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
    let output = lab
        .execute(&format!(r#"{{"op":"output","job_id":"{}"}}"#, job_id))
        .await
        .unwrap();
    assert!(output.is_error, "{}", output.content);
}

#[tokio::test]
async fn concurrent_foreground_run_keeps_its_artifacts_while_others_prune() {
    // Foreground runs have no job-registry entry, so pruning used to delete
    // their artifact directory mid-run and the run then failed hard on ENOENT.
    let tmp = tempfile::tempdir().unwrap();
    let lab = Arc::new(tool(tmp.path()));
    let slow = {
        let lab = lab.clone();
        tokio::spawn(async move {
            lab.execute(r#"{"op":"run","code":"import time; time.sleep(1.5); print('survived')","timeout_seconds":2}"#)
                .await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    for _ in 0..40 {
        lab.execute(r#"{"op":"run","code":"pass"}"#).await.unwrap();
    }
    let result = slow.await.unwrap().expect("slow run should not error");
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["status"], "completed", "{}", result.content);
    assert!(
        v["stdout"].as_str().unwrap().starts_with("survived"),
        "slow run lost its output to pruning: {}",
        result.content
    );
}

#[tokio::test]
async fn cancel_during_result_build_is_not_overwritten() {
    // The terminal status is published after the result is built; a cancel
    // landing in that window must not be clobbered back to "completed".
    let tmp = tempfile::tempdir().unwrap();
    let lab = tool(tmp.path());
    let started = lab
        .execute(r#"{"op":"run","code":"print('done')","background":true}"#)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&started.content).unwrap();
    let job_id = v["job_id"].as_str().unwrap().to_string();
    // Cancel repeatedly across the whole run/build window.
    for _ in 0..60 {
        let status = lab
            .execute(&format!(r#"{{"op":"status","job_id":"{job_id}"}}"#))
            .await
            .unwrap();
        let s: serde_json::Value = serde_json::from_str(&status.content).unwrap();
        let observed = s["status"].as_str().unwrap_or_default().to_string();
        if observed == "cancelling" {
            // Once a cancel is acknowledged the job must settle as cancelled.
            for _ in 0..50 {
                let later = lab
                    .execute(&format!(r#"{{"op":"status","job_id":"{job_id}"}}"#))
                    .await
                    .unwrap();
                let l: serde_json::Value = serde_json::from_str(&later.content).unwrap();
                match l["status"].as_str().unwrap_or_default() {
                    "cancelling" => {}
                    "cancelled" => return,
                    other => panic!("acknowledged cancel settled as {other:?}: {later:?}"),
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            panic!("cancelling never settled");
        }
        if observed == "completed" {
            return; // finished before any cancel was acknowledged
        }
        let _ = lab
            .execute(&format!(r#"{{"op":"cancel","job_id":"{job_id}"}}"#))
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
}
