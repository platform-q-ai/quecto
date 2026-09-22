//! The owner's exit announcement over a real connection (#2070): raised on
//! the reader task while the dispatch loop is blocked, still forwarded to
//! it, untouched by any other persist, and withdrawn when the announcing
//! connection closes.
use std::sync::Arc;

use tokio::io::AsyncWriteExt;

use super::super::spawn_accept_loop;
use super::{make_args, read_available};
use crate::application::subagents::ports::OwnerExitAnnouncement;

async fn until(mut condition: impl FnMut() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !condition() {
        assert!(std::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn the_exit_persist_announces_on_the_reader_task_and_its_close_withdraws() {
    let dir = tempfile::tempdir().expect("tempdir");
    let socket_path = dir.path().join("owner-exit.sock");
    let registry = crate::infrastructure::tools::subagent_registry::new_registry();
    // The dispatch channel is never drained: a turn that has not ended.
    let (mut args, bcast, _cmd_tx, mut cmd_rx) =
        make_args(&socket_path, true, Some(registry.clone()));
    let flag = crate::infrastructure::tools::owner_exit::OwnerExitFlag::new();
    let graph = crate::composition::subagent_teardown::build_teardown_graph(
        crate::interface::cli::uds_teardown_handles::TeardownLoopInputs {
            owner: crate::domain::ids::AgentUuid::new("root"),
            registry: Some(registry),
            harness_lifecycle: None,
            broadcast_tx: Some(bcast),
            notify_tx: None,
            cancel_handle: args.cancel_handle.clone(),
            turn_control: args.turn_control.clone(),
            busy: args.busy.clone(),
            exit_notify: Arc::new(tokio::sync::Notify::new()),
            binding: crate::domain::parent_control::ParentControlBinding::unlaunched(),
            owner_exit: flag.clone(),
            environment_control: None,
        },
    );
    args.teardown = Some(graph.connections.clone());
    let handle = spawn_accept_loop(args);

    // Another client's routine persist announces nothing.
    let mut other = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
    let _ = read_available(&mut other, std::time::Duration::from_millis(200)).await;
    other
        .write_all(b"{\"type\":\"persist_session\",\"id\":\"routine\"}\n")
        .await
        .unwrap();
    until(
        || cmd_rx.try_recv().is_ok(),
        "the routine persist reaches the dispatch queue",
    )
    .await;
    assert!(!flag.announced());

    // The owner's exit persist: raised before the dispatch loop could run.
    let mut owner = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
    let _ = read_available(&mut owner, std::time::Duration::from_millis(200)).await;
    owner
        .write_all(b"{\"type\":\"persist_session\",\"id\":\"persist-exit-1\",\"restoreReason\":\"ordinary_tui_exit_stopped\"}\n")
        .await
        .unwrap();
    until(|| flag.announced(), "announced on the reader task").await;
    until(
        || cmd_rx.try_recv().is_ok(),
        "the exit persist is still forwarded for the save",
    )
    .await;

    // The other client leaving changes nothing; the announcer leaving withdraws.
    drop(other);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        flag.announced(),
        "another client's close leaves the announcement"
    );
    drop(owner);
    until(
        || !flag.announced(),
        "withdrawn when the announcing connection closes",
    )
    .await;
    handle.abort();
}
