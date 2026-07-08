// #831: kill cascade-removes the dead sub-tree from the registry and broadcasts
// the survivor-only `subagent_state_changed` so connected clients (the TUI
// panel) drop the whole dead sub-tree promptly instead of letting it linger.
use super::*;
use std::path::PathBuf;

fn child_entry(parent: &str) -> SubagentEntry {
    let mut e = SubagentEntry::new(PathBuf::from("/tmp/x.sock"), 0);
    e.parent_id = Some(parent.to_string());
    e
}

#[test]
fn kill_cascade_removes_subtree_and_broadcasts_survivors() {
    let registry = new_registry();
    {
        let mut g = registry.lock().unwrap();
        // parent → child → grandchild, plus an unrelated live sibling.
        g.insert(
            "parent".to_string(),
            SubagentEntry::new(PathBuf::from("/tmp/x.sock"), 0),
        );
        g.insert("child".to_string(), child_entry("parent"));
        g.insert("gchild".to_string(), child_entry("child"));
        g.insert(
            "live".to_string(),
            SubagentEntry::new(PathBuf::from("/tmp/x.sock"), 0),
        );
    }
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    let tool = AgentCmdTool::new(registry.clone()).with_broadcast(Some(tx));

    let result = tool.kill_agent("parent");
    assert!(!result.is_error, "kill should succeed: {}", result.content);

    // Whole dead sub-tree pruned; the live sibling is untouched.
    let g = registry.lock().unwrap();
    assert!(!g.contains_key("parent"));
    assert!(!g.contains_key("child"));
    assert!(!g.contains_key("gchild"));
    assert!(g.contains_key("live"), "live agent must never be removed");
    drop(g);

    // A survivor-only state_changed was broadcast. #1055: the payload SENT
    // from this site must be a single newline-terminated, parseable line.
    let event = rx.try_recv().expect("a state_changed should be broadcast");
    assert!(
        event.ends_with('\n'),
        "sent broadcast must end with newline"
    );
    serde_json::from_str::<serde_json::Value>(&event).expect("sent line parses");
    assert!(event.contains("subagent_state_changed"));
    assert!(event.contains("live"));
    assert!(!event.contains("parent"));
    assert!(!event.contains("gchild"));
}

#[tokio::test]
async fn kill_signals_await_aborts_monitor_and_sigterms_live_pid() {
    use crate::infrastructure::tools::subagent_registry::new_exit_signal_channel;
    // A real child so the SIGTERM (pid != 0) branch runs against a live pid.
    let child = std::process::Command::new("sleep")
        .arg("30")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn sleep");
    let pid = child.id();

    let registry = new_registry();
    let (exit_tx, _exit_rx) = new_exit_signal_channel();
    let monitor = std::sync::Arc::new(tokio::spawn(async {
        std::future::pending::<()>().await;
    }));
    {
        let mut g = registry.lock().unwrap();
        let mut e = SubagentEntry::new(PathBuf::from("/tmp/x.sock"), pid);
        e.exit_signal_tx = Some(exit_tx.clone());
        e.monitor_handle = Some(monitor.clone());
        g.insert("solo".to_string(), e);
    }
    // A subscribed await receiver must be signalled with the SIGTERM exit.
    let mut await_rx = exit_tx.subscribe();
    let (tx, _rx) = tokio::sync::broadcast::channel::<String>(8);
    let tool = AgentCmdTool::new(registry.clone()).with_broadcast(Some(tx));

    let result = tool.kill_agent("solo");
    assert!(!result.is_error);
    assert!(result.content.contains(&pid.to_string()));

    // await was signalled with the SIGTERM exit, the monitor was aborted.
    let signal = await_rx.borrow_and_update().clone();
    assert_eq!(signal.and_then(|s| s.signal), Some(15));
    // abort() schedules cancellation; let the runtime drive it to completion.
    for _ in 0..100 {
        if monitor.is_finished() {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(monitor.is_finished(), "monitor task should be aborted");
    assert!(registry.lock().unwrap().is_empty());

    // Reap the sleep child (kill_agent SIGTERMed it; ensure no zombie).
    let mut child = child;
    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test]
async fn kill_parent_sigterms_descendant_processes_not_just_named_agent() {
    // #831 security review: killing a parent must SIGTERM its DESCENDANTS' OS
    // processes too, not merely drop them from the registry — otherwise they
    // linger as untracked orphans that shutdown_all can no longer reach.
    let spawn_sleep = || {
        std::process::Command::new("sleep")
            .arg("30")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn sleep")
    };
    let mut parent_proc = spawn_sleep();
    let mut gchild_proc = spawn_sleep();
    let parent_pid = parent_proc.id();
    let gchild_pid = gchild_proc.id();

    let registry = new_registry();
    {
        let mut g = registry.lock().unwrap();
        g.insert(
            "parent".to_string(),
            SubagentEntry::new(PathBuf::from("/tmp/x.sock"), parent_pid),
        );
        let mut gc = child_entry("parent");
        gc.pid = gchild_pid;
        g.insert("gchild".to_string(), gc);
    }
    let (tx, _rx) = tokio::sync::broadcast::channel::<String>(8);
    let tool = AgentCmdTool::new(registry.clone()).with_broadcast(Some(tx));

    let result = tool.kill_agent("parent");
    assert!(!result.is_error);
    assert!(registry.lock().unwrap().is_empty());

    // Both the parent AND the descendant process received SIGTERM and exit.
    // (Reap to confirm; without the descendant SIGTERM, gchild would still be
    // sleeping and this wait would block past the loop.)
    let parent_status = parent_proc.wait().expect("reap parent");
    let mut gchild_exited = false;
    for _ in 0..200 {
        match gchild_proc.try_wait() {
            Ok(Some(_)) => {
                gchild_exited = true;
                break;
            }
            _ => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
        }
    }
    let _ = parent_status;
    assert!(
        gchild_exited,
        "descendant process must be SIGTERMed when its parent is killed"
    );
    let _ = gchild_proc.kill();
    let _ = gchild_proc.wait();
}

#[test]
fn kill_unknown_agent_returns_error_and_no_broadcast() {
    let registry = new_registry();
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    let tool = AgentCmdTool::new(registry).with_broadcast(Some(tx));
    let result = tool.kill_agent("ghost");
    assert!(result.is_error);
    assert!(result.content.contains("not found"));
    assert!(rx.try_recv().is_err(), "no broadcast for unknown agent");
}
