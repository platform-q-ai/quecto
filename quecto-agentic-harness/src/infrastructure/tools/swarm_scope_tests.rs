//! Ordinary descendants belong to an invocation even when its interpreter exits.
use super::swarm::{SwarmConfig, SwarmTool};
use super::swarm_bridge::SwarmContext;
use crate::application::tools::ports::Tool;
use crate::infrastructure::security::sandbox::Sandbox;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn foreground_descendants_stop_before_cancelled_run_is_reported() {
    settled_descendants(false, false).await;
}
#[tokio::test]
async fn background_descendants_stop_before_cancelled_run_is_reported() {
    settled_descendants(true, false).await;
}
#[tokio::test]
async fn foreground_descendants_stop_before_successful_run_is_reported() {
    settled_descendants(false, true).await;
}
#[tokio::test]
async fn background_descendants_stop_before_successful_run_is_reported() {
    settled_descendants(true, true).await;
}

async fn settled_descendants(background: bool, success: bool) {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = Arc::new(tmp.path().to_path_buf());
    std::fs::create_dir_all(workspace.join(".quecto")).unwrap();
    let context = SwarmContext {
        checkout: workspace.as_ref().clone(),
        member: "coordinator".into(),
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
    };
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 120;
    context.call("create", json!(["settled checkout",[],[{"id":"tests","kind":"command","description":"pass"}],1,deadline])).unwrap();
    let tool = SwarmTool::new(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
        SwarmConfig {
            max_processes: None,
            ..Default::default()
        },
    )
    .with_context(Some(context.clone()));
    let child = "import pathlib,time,os\npathlib.Path('child-ready.tmp').write_text(str(os.getpid()))\nos.replace('child-ready.tmp','child-ready')\nend=time.monotonic()+10\nwhile not pathlib.Path('release-child').exists() and time.monotonic()<end: time.sleep(0.01)\nif pathlib.Path('release-child').exists(): pathlib.Path('late-write').write_text('escaped')";
    let code = format!(
        "import pathlib,subprocess,sys,time\nsubprocess.Popen([sys.executable,'-c',{}],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\nwhile not pathlib.Path('child-ready').exists(): time.sleep(0.01)",
        json!(child)
    );
    let result = tool
        .execute(&json!({"code":code,"background":background,"timeout_seconds":20}).to_string())
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    if background {
        let started: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let status = tool
                    .execute(&json!({"op":"status","job_id":started["job_id"]}).to_string())
                    .await
                    .unwrap();
                let status: serde_json::Value = serde_json::from_str(&status.content).unwrap();
                if status["status"] == "completed" {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    if success {
        let done = tool.execute(r#"{"code":"from swarm import board; board.evidence('tests','proof','R1','command',True); board.complete('R1')"}"#).await.unwrap();
        assert!(!done.is_error, "{}", done.content);
    } else {
        assert!(
            !tool
                .execute(r#"{"op":"cancel_run"}"#)
                .await
                .unwrap()
                .is_error
        );
    }
    assert_eq!(
        context.summary().unwrap()["status"],
        if success { "paused" } else { "cancelled" },
        "completion ends the run as a resumable pause (#1729)"
    );
    let pid = wait_for_child_pid(&workspace).await;
    std::fs::write(workspace.join("release-child"), "go").unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while process_running(pid) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("ordinary child survived invocation settlement");
    assert!(
        !workspace.join("late-write").exists(),
        "completed invocation left a writer after run settlement"
    );
}

/// The pid the Python child publishes as `child-ready`. The child writes
/// `child-ready.tmp` and renames it into place, so the file is either absent
/// or complete; the reader still only trusts a value that parses, and the
/// ceiling merely bounds a hang so it is generous to loaded CI runners.
async fn wait_for_child_pid(workspace: &std::path::Path) -> u32 {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Some(pid) = std::fs::read_to_string(workspace.join("child-ready"))
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok())
            {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("python child never published its pid")
}

fn process_running(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|s| {
            s.rsplit_once(')')
                .map(|(_, tail)| !tail.trim_start().starts_with('Z'))
        })
        .unwrap_or(false)
}

#[tokio::test]
async fn timeout_stops_ordinary_descendants_of_a_live_interpreter() {
    interrupted_descendants(false).await;
}

#[tokio::test]
async fn dropped_invocation_stops_ordinary_descendants_of_a_live_interpreter() {
    interrupted_descendants(true).await;
}

async fn interrupted_descendants(drop_invocation: bool) {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = Arc::new(tmp.path().to_path_buf());
    let tool = super::swarm_test_support::tool(
        workspace.clone(),
        Arc::new(Sandbox::new(Some(workspace.as_ref().clone()))),
        SwarmConfig {
            max_processes: None,
            ..Default::default()
        },
    );
    let child = "import pathlib,time,os; pathlib.Path('child-ready.tmp').write_text(str(os.getpid())); os.replace('child-ready.tmp','child-ready'); time.sleep(10)";
    let code = format!(
        "import subprocess,sys,time; subprocess.Popen([sys.executable,'-c',{}],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); time.sleep(10)",
        json!(child)
    );
    let invocation = tokio::spawn(async move {
        tool.execute(
            &json!({"code":code,"timeout_seconds":if drop_invocation { 20 } else { 1 }})
                .to_string(),
        )
        .await
        .unwrap()
    });
    let pid = wait_for_child_pid(&workspace).await;
    if drop_invocation {
        invocation.abort();
        assert!(invocation.await.unwrap_err().is_cancelled());
    } else {
        let result = invocation.await.unwrap();
        assert!(
            result.is_error && result.content.contains("timed_out"),
            "{}",
            result.content
        );
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while process_running(pid) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("interrupted invocation retained an ordinary child");
}
