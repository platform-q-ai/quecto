//! A pre-ready exit carries the child's last stderr (#1937 review): the
//! reason a launched child refused to start is visible to its launcher.
use super::SpawnTool;
use crate::infrastructure::tools::spawn_container::PreparedChild;

fn sh(script: &str) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("sh");
    command.arg("-c").arg(script);
    command
}

#[tokio::test]
async fn pre_ready_exit_reports_the_childs_stderr_tail() {
    let dir = tempfile::TempDir::new().unwrap();
    let prepared = PreparedChild::new_for_test(
        Some(sh(
            "echo 'agent: --persist is refused with --parent-control' >&2; exit 1",
        )),
        None,
        None,
    )
    .await;
    let err = SpawnTool::new(vec![])
        .wait_for_socket_or_child_exit(&dir.path().join("never.sock"), &prepared)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.ends_with(
            "subagent exited before socket ready with Code(1); stderr: agent: --persist is refused with --parent-control"
        ),
        "got: {err}"
    );
}

#[tokio::test]
async fn pre_ready_exit_without_stderr_reports_the_status_only() {
    let dir = tempfile::TempDir::new().unwrap();
    let prepared = PreparedChild::new_for_test(Some(sh("exit 3")), None, None).await;
    let err = SpawnTool::new(vec![])
        .wait_for_socket_or_child_exit(&dir.path().join("never.sock"), &prepared)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.ends_with("subagent exited before socket ready with Code(3)"),
        "got: {err}"
    );
}
