use super::app_subagents_tests::{harness, info, info_with_parent, info_with_parent_and_socket};
#[tokio::test]
async fn source_scoped_roster_accepts_recursive_descendants_in_one_event() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("a", "running")]);

    a.update_subagent_bar_from_source(
        Some("a"),
        vec![
            info_with_parent("a1", "running", "a"),
            info_with_parent("g1", "running", "a1"),
        ],
    );

    assert_eq!(
        a.subagents.tracked["g1"].info.parent_id.as_deref(),
        Some("a1")
    );
}

#[tokio::test]
async fn source_scoped_roster_cannot_reparent_existing_root() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("a", "running"), info("b", "running")]);

    a.update_subagent_bar_from_source(Some("a"), vec![info_with_parent("b", "idle", "a")]);

    assert_eq!(a.subagents.tracked["b"].info.parent_id, None);
}

#[tokio::test]
async fn direct_child_metadata_survives_later_master_snapshot() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("a", "running")]);
    a.update_subagent_bar_from_source(Some("a"), vec![info_with_parent("a1", "idle", "a")]);

    a.update_subagent_bar(vec![
        info("a", "running"),
        info_with_parent("a1", "running", "a"),
    ]);

    assert_eq!(a.subagents.tracked["a1"].info.status, "idle");
}

use super::tui_harness::*;
use crate::protocol::client::Event;

#[tokio::test]
async fn recursive_discovery_registers_grandchild_and_opens_warm_feed() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("a", "running")]);
    a.update_subagent_bar_from_source(Some("a"), vec![info_with_parent("a1", "running", "a")]);

    let (socket, mut cmd_rx) = spawn_subagent_socket_with_commands("g1");
    a.route_subagent_event(
        "a1",
        Event::SubagentStateChanged {
            subagents: vec![info_with_parent_and_socket(
                "g1",
                "running",
                "a1",
                Some(&socket.to_string_lossy()),
            )],
        },
    );

    assert_eq!(
        a.subagents.tracked["g1"].info.parent_id.as_deref(),
        Some("a1")
    );
    assert_eq!(
        a.subagents.tracked["g1"].info.socket_path.as_deref(),
        Some(socket.to_string_lossy().as_ref())
    );

    let commands = wait_for_child_commands(&mut cmd_rx, |commands| {
        commands
            .iter()
            .any(|line| line.contains(r#""type":"sync""#))
    })
    .await;
    assert!(
        commands.iter().any(|line| {
            line.contains(r#""type":"sync""#)
                && line.contains(r#""epoch":0"#)
                && line.contains(r#""sinceRev":0"#)
        }),
        "recursive discovery must open a warm synced feed and request an initial ledger sync; got {commands:?}"
    );
    assert_eq!(
        a.subagents.feeds["g1"].authority,
        crate::interface::agents::feed::FeedAuthority::WarmSync,
        "warm feeds must not suppress legacy child events until sync support is confirmed"
    );
}

#[tokio::test]
async fn selecting_warm_synced_agent_reuses_existing_feed() {
    let (socket, mut child_rx) = spawn_subagent_socket_with_commands("warm-focus");
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent_with_socket(
        "warm-focus",
        "running",
        Some(("active", 1, 3)),
        Some(socket),
    )]));

    let initial = wait_for_child_commands(&mut child_rx, |commands| {
        commands
            .iter()
            .filter(|line| line.contains(r#""type":"sync""#))
            .count()
            == 1
    })
    .await;
    assert_eq!(
        initial
            .iter()
            .filter(|line| line.contains(r#""type":"sync""#))
            .count(),
        1,
        "discovery should open exactly one warm sync feed before focus: {initial:?}"
    );

    h.app_mut()
        .note_sync_capability("warm-focus", &serde_json::json!({"sync":1}));
    h.app_mut().route_sync_response(
        "warm-focus",
        &serde_json::json!({
            "epoch": 1,
            "rev": 1,
            "messages": [],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    h.select(Some("warm-focus"));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), child_rx.recv())
            .await
            .is_err(),
        "focus must reuse a synced authoritative warm feed instead of reconnecting and sending another command"
    );
}

#[tokio::test]
async fn selecting_warm_unsynced_agent_does_not_reopen_legacy_backfill_feed() {
    let (socket, mut child_rx) = spawn_subagent_socket_with_commands("legacy-focus");
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent_with_socket(
        "legacy-focus",
        "running",
        Some(("active", 1, 3)),
        Some(socket),
    )]));

    let initial = wait_for_child_commands(&mut child_rx, |commands| {
        commands
            .iter()
            .any(|line| line.contains(r#""type":"sync""#))
    })
    .await;
    assert!(
        initial.iter().any(|line| line.contains(r#""type":"sync""#)),
        "discovery should try ledger sync: {initial:?}"
    );

    h.select(Some("legacy-focus"));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), child_rx.recv())
            .await
            .is_err(),
        "focus must not reopen the deleted legacy get_messages backfill path"
    );
}

#[tokio::test]
async fn stale_synced_focus_requests_exactly_one_catch_up_sync() {
    let (socket, mut child_rx) = spawn_subagent_socket_with_commands("stale-focus");
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent_with_socket(
        "stale-focus",
        "running",
        Some(("active", 1, 3)),
        Some(socket),
    )]));

    let initial = wait_for_child_commands(&mut child_rx, |commands| {
        commands
            .iter()
            .any(|line| line.contains(r#""type":"sync""#))
    })
    .await;
    assert!(initial.iter().any(|line| line.contains(r#""type":"sync""#)));

    h.app_mut()
        .note_sync_capability("stale-focus", &serde_json::json!({"sync":1}));
    h.app_mut().route_sync_response(
        "stale-focus",
        &serde_json::json!({
            "epoch": 7,
            "rev": 11,
            "messages": [],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );
    h.app_mut()
        .subagents
        .feeds
        .get_mut("stale-focus")
        .unwrap()
        .last_fresh_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(2));

    h.select(Some("stale-focus"));
    let selected = wait_for_child_commands(&mut child_rx, |commands| {
        commands
            .iter()
            .filter(|line| line.contains(r#""type":"sync""#))
            .count()
            == 1
    })
    .await;
    assert_eq!(
        selected
            .iter()
            .filter(|line| {
                line.contains(r#""type":"sync""#)
                    && line.contains(r#""epoch":7"#)
                    && line.contains(r#""sinceRev":11"#)
            })
            .count(),
        1,
        "stale focus must request exactly one authoritative catch-up sync: {selected:?}"
    );
}

async fn wait_for_child_commands<F>(
    rx: &mut tokio::sync::mpsc::Receiver<String>,
    done: F,
) -> Vec<String>
where
    F: Fn(&[String]) -> bool,
{
    let mut commands = Vec::new();
    for _ in 0..200 {
        while let Ok(line) = rx.try_recv() {
            commands.push(line);
        }
        if done(&commands) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    commands
}
