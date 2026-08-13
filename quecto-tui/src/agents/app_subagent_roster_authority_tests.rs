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
        a.ac().roster.tracked["g1"].info.parent_id.as_deref(),
        Some("a1")
    );
}

#[tokio::test]
async fn source_scoped_roster_cannot_reparent_existing_root() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_subagent_bar(vec![info("a", "running"), info("b", "running")]);

    a.update_subagent_bar_from_source(Some("a"), vec![info_with_parent("b", "idle", "a")]);

    assert_eq!(a.ac().roster.tracked["b"].info.parent_id, None);
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

    assert_eq!(a.ac().roster.tracked["a1"].info.status, "idle");
}

use super::tui_harness::*;
use crate::protocol::client::Event;

fn is_sync_cmd(line: &str) -> bool {
    child_command_type(line).as_deref() == Some("sync")
}

fn is_get_messages_cmd(line: &str) -> bool {
    child_command_type(line).as_deref() == Some("get_messages")
}

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
        a.ac().roster.tracked["g1"].info.parent_id.as_deref(),
        Some("a1")
    );
    assert_eq!(
        a.ac().roster.tracked["g1"].info.socket_path.as_deref(),
        Some(socket.to_string_lossy().as_ref())
    );

    let commands = drain_child_commands_until_quiet(&mut cmd_rx).await;
    assert!(
        commands.iter().any(|line| {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                return false;
            };
            v.get("type").and_then(|t| t.as_str()) == Some("sync")
                && v.get("epoch").and_then(|e| e.as_u64()) == Some(0)
                && v.get("sinceRev").and_then(|r| r.as_u64()) == Some(0)
        }),
        "recursive discovery must open a warm synced feed and request an initial ledger sync; got {commands:?}"
    );
    assert_eq!(
        a.ac().roster.feeds["g1"].authority,
        crate::agents::feed::FeedAuthority::WarmSync,
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

    let initial = drain_child_commands_until_quiet(&mut child_rx).await;
    assert_eq!(
        initial.iter().filter(|line| is_sync_cmd(line)).count(),
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

    let feed_cmd = h
        .app_mut()
        .ac()
        .roster
        .feeds
        .get("warm-focus")
        .expect("warm feed opened on discovery")
        .cmd_tx
        .clone();

    h.select(Some("warm-focus"));
    assert_no_further_child_commands(
        &mut child_rx,
        "focus must reuse a synced authoritative warm feed instead of reconnecting",
    )
    .await;
    let still = h
        .app_mut()
        .ac()
        .roster
        .feeds
        .get("warm-focus")
        .expect("feed must remain after focus");
    assert!(
        still.cmd_tx.same_channel(&feed_cmd),
        "focus must keep the existing feed runtime rather than replacing it"
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

    let initial = drain_child_commands_until_quiet(&mut child_rx).await;
    assert!(
        initial.iter().any(|line| is_sync_cmd(line)),
        "discovery should try ledger sync: {initial:?}"
    );
    assert!(
        !initial.iter().any(|line| is_get_messages_cmd(line)),
        "warm discovery must not use legacy get_messages backfill: {initial:?}"
    );

    let feed_cmd = h
        .app_mut()
        .ac()
        .roster
        .feeds
        .get("legacy-focus")
        .expect("warm feed opened on discovery")
        .cmd_tx
        .clone();
    let feeds_before = h.app_mut().ac().roster.feeds.len();

    h.select(Some("legacy-focus"));
    let after_focus = drain_child_commands_until_quiet(&mut child_rx).await;
    assert!(
        after_focus.iter().all(|line| !is_get_messages_cmd(line)),
        "focus must not reopen the deleted legacy get_messages backfill path; got {after_focus:?}"
    );
    assert!(
        after_focus.iter().all(|line| !is_sync_cmd(line)),
        "unsynced warm focus must not re-issue startup sync; got {after_focus:?}"
    );
    assert_eq!(
        h.app_mut().ac().roster.feeds.len(),
        feeds_before,
        "focus must not open an additional feed entry"
    );
    let still = h
        .app_mut()
        .ac()
        .roster
        .feeds
        .get("legacy-focus")
        .expect("warm feed must remain after focus");
    assert!(
        still.cmd_tx.same_channel(&feed_cmd),
        "focus must reuse the existing warm feed runtime instead of reconnecting"
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

    let initial = drain_child_commands_until_quiet(&mut child_rx).await;
    assert!(
        initial.iter().any(|line| is_sync_cmd(line)),
        "discovery should try ledger sync: {initial:?}"
    );

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
        .ac_mut()
        .roster
        .feeds
        .get_mut("stale-focus")
        .unwrap()
        .last_fresh_at = Some(std::time::Instant::now() - std::time::Duration::from_secs(2));

    h.select(Some("stale-focus"));
    let selected = drain_child_commands_until_quiet(&mut child_rx).await;
    let catch_ups: Vec<_> = selected
        .iter()
        .filter(|line| {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                return false;
            };
            v.get("type").and_then(|t| t.as_str()) == Some("sync")
                && v.get("epoch").and_then(|e| e.as_u64()) == Some(7)
                && v.get("sinceRev").and_then(|r| r.as_u64()) == Some(11)
        })
        .collect();
    assert_eq!(
        catch_ups.len(),
        1,
        "stale focus must request exactly one authoritative catch-up sync: {selected:?}"
    );
    assert_no_further_child_commands(
        &mut child_rx,
        "stale focus must not emit a second catch-up sync after settle",
    )
    .await;
}
