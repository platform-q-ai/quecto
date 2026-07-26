//! Characterization tests for the agents presentation extraction (#1222).
//!
//! These tests pin cross-cutting roster, feed, ledger, and focus behaviour before
//! production code is moved into the agents module. They drive the real App and
//! command channels rather than asserting on source layout.

use super::*;
use serde_json::json;
use std::time::Duration;

fn subagent(id: &str, status: &str) -> crate::infrastructure::client::SubagentInfoEvent {
    crate::infrastructure::client::SubagentInfoEvent {
        agent_id: id.to_string(),
        status: status.to_string(),
        last_tool: None,
        last_error: None,
        pid: 0,
        socket_path: None,
        parent_id: None,
        workflow: None,
        read_only: false,
    }
}

fn child(id: &str, status: &str, parent: &str) -> crate::infrastructure::client::SubagentInfoEvent {
    let mut info = subagent(id, status);
    info.parent_id = Some(parent.to_string());
    info
}

fn feed_with_rx() -> (FeedState, mpsc::Receiver<Command>) {
    let (cmd_tx, cmd_rx) = mpsc::channel(8);
    let handle = tokio::spawn(async {});
    (
        FeedState {
            cmd_tx,
            handle,
            epoch: 0,
            rev: 0,
            last_fresh_at: None,
            supports_sync: false,
            pending_rev: None,
            transcript: crate::interface::agents::ledger::LedgerTranscript::default(),
            authority: crate::interface::agents::feed::FeedAuthority::WarmSync,
        },
        cmd_rx,
    )
}

#[tokio::test]
async fn warm_feed_startup_sends_get_state_then_initial_sync_once() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let (socket, mut child_commands) =
        super::tui_harness::spawn_subagent_socket_with_commands("worker");
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent_with_socket("worker", "running", None, Some(socket)),
    ]));

    let mut commands = Vec::new();
    for _ in 0..20 {
        while let Ok(line) = child_commands.try_recv() {
            commands.push(line);
        }
        if commands.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
    while let Ok(line) = child_commands.try_recv() {
        commands.push(line);
    }
    assert_eq!(
        commands.len(),
        2,
        "warm direct feed startup must send exactly get_state then sync once, got {commands:?}"
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&commands[0]).unwrap()["type"],
        "get_state"
    );
    let sync = serde_json::from_str::<serde_json::Value>(&commands[1]).unwrap();
    assert_eq!(sync["type"], "sync");
    assert_eq!(sync["epoch"], 0);
    assert_eq!(sync["sinceRev"], 0);

    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent_with_socket(
            "worker",
            "running",
            None,
            Some(std::path::PathBuf::from("/invalid/reconnect/socket")),
        ),
    ]));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        child_commands.try_recv().is_err(),
        "a later roster update for an already-warmed feed must not reconnect or repeat startup commands"
    );
}

#[tokio::test]
async fn retained_sessions_and_warm_feeds_evict_oldest_non_active_beyond_cap() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    app.subagents.active_agent_id = Some("agent-00".into());
    for i in 0..17 {
        let id = format!("agent-{i:02}");
        let (feed, _rx) = feed_with_rx();
        app.subagents.feeds.insert(id.clone(), feed);
        app.ensure_session(&id);
    }

    assert_eq!(app.subagents.sessions.len(), 16);
    assert_eq!(app.subagents.feeds.len(), 16);
    assert!(
        app.subagents.sessions.contains_key("agent-00"),
        "active session must be preserved even when it is oldest"
    );
    assert!(
        app.subagents.feeds.contains_key("agent-00"),
        "active feed must be preserved with its active session"
    );
    assert!(
        !app.subagents.sessions.contains_key("agent-01"),
        "the oldest non-active retained session should be evicted first"
    );
    assert!(
        !app.subagents.feeds.contains_key("agent-01"),
        "evicting a retained session must also clean up its warm feed"
    );
    assert!(app.subagents.sessions.contains_key("agent-16"));
}

#[test]
fn duplicate_ids_within_one_sync_delta_keep_first_position_and_latest_content() {
    let mut transcript = crate::interface::agents::ledger::LedgerTranscript::default();

    let entries =
        transcript.apply_sync_delta(&crate::application::agent_ledger_payloads::SyncDelta {
            epoch: 1,
            rev: 1,
            messages: vec![
                serde_json::from_value(json!({"id":"dup","role":"assistant","content":"old"}))
                    .unwrap(),
                serde_json::from_value(json!({"id":"other","role":"user","content":"between"}))
                    .unwrap(),
                serde_json::from_value(json!({"id":"dup","role":"assistant","content":"new"}))
                    .unwrap(),
            ],
            next_rev: None,
            caught_up: true,
            resync: false,
        });

    assert!(
        matches!(
            entries.as_slice(),
            [
                crate::interface::agents::ledger::LedgerEntry::Assistant { text: first },
                crate::interface::agents::ledger::LedgerEntry::User { text: second }
            ] if first == "new" && second == "between"
        ),
        "duplicate ids must update in place without moving their original order slot: {entries:?}"
    );
}

#[tokio::test]
async fn ledger_hint_at_current_revision_records_freshness_without_redundant_sync() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, mut rx) = feed_with_rx();
    feed.supports_sync = true;
    feed.epoch = 7;
    feed.rev = 11;
    app.subagents.feeds.insert("worker".into(), feed);

    app.note_ledger_advanced("worker", 7, 11);

    assert!(
        rx.try_recv().is_err(),
        "a ledger hint at the already-applied revision must not request a duplicate sync"
    );
    let feed = app.subagents.feeds.get_mut("worker").unwrap();
    assert_eq!(feed.epoch, 7);
    assert_eq!(feed.rev, 11);
    assert_eq!(feed.pending_rev, None);
    assert!(
        feed.last_fresh_at.is_some(),
        "even a no-op hint refreshes feed freshness"
    );
    feed.authority = crate::interface::agents::feed::FeedAuthority::SyncedAuthoritative;

    app.select_agent(Some("worker"));

    assert!(
        rx.try_recv().is_err(),
        "a current-revision hint must keep focus refresh from requesting a redundant sync"
    );
}

#[tokio::test]
async fn caught_up_sync_clears_pending_revision_without_follow_up_command() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, mut rx) = feed_with_rx();
    feed.supports_sync = true;
    feed.pending_rev = Some(9);
    app.subagents.feeds.insert("worker".into(), feed);
    app.ensure_session("worker");

    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 3,
            "rev": 9,
            "messages": [{"id":"u1","role":"user","content":"ledger text"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    assert!(
        rx.try_recv().is_err(),
        "a caught-up sync delta must not enqueue a continuation request"
    );
    let feed = app.subagents.feeds.get("worker").unwrap();
    assert_eq!(feed.epoch, 3);
    assert_eq!(feed.rev, 9);
    assert_eq!(feed.pending_rev, None);
    assert_eq!(
        feed.authority,
        crate::interface::agents::feed::FeedAuthority::SyncedAuthoritative,
        "authority changes only after applying a valid sync delta"
    );
    let entries = app.subagents.sessions["worker"].chat.entries();
    assert!(matches!(entries, [ChatEntry::User { text }] if text == "ledger text"));
}

#[tokio::test]
async fn active_child_removed_by_its_source_feed_falls_back_to_master_only() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    app.update_subagent_bar(vec![
        subagent("parent", "running"),
        subagent("sibling", "running"),
    ]);
    app.update_subagent_bar_from_source(
        Some("parent"),
        vec![
            child("child", "running", "parent"),
            child("grandchild", "running", "child"),
        ],
    );
    app.select_agent(Some("child"));
    assert_eq!(app.active_agent_id(), Some("child"));

    app.update_subagent_bar_from_source(Some("parent"), vec![]);

    assert_eq!(
        app.active_agent_id(),
        None,
        "removing the active child through its authoritative source feed must fall back to master"
    );
    assert!(
        app.subagents.tracked.contains_key("parent"),
        "source-scoped removal must not drop the source root itself"
    );
    assert!(
        app.subagents.tracked.contains_key("sibling"),
        "source-scoped removal must preserve unrelated master-owned siblings"
    );
    assert!(!app.subagents.tracked.contains_key("child"));
    assert!(!app.subagents.tracked.contains_key("grandchild"));
}

#[tokio::test]
async fn unfocused_authoritative_ledger_projection_suppresses_legacy_live_child_tokens() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, _rx) = feed_with_rx();
    feed.supports_sync = true;
    app.subagents.feeds.insert("worker".into(), feed);
    app.ensure_session("worker");
    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 1,
            "rev": 1,
            "messages": [{"id":"a1","role":"assistant","content":"from ledger"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    app.route_subagent_event(
        "worker",
        Event::Token {
            token: "legacy duplicate".into(),
        },
    );

    let entries = app.subagents.sessions["worker"].chat.entries();
    assert!(
        matches!(entries, [ChatEntry::Assistant { text, streaming: false }] if text == "from ledger"),
        "once sync is authoritative for an unfocused child, legacy live tokens must not duplicate the ledger transcript: {entries:?}"
    );
}

#[tokio::test]
async fn focused_authoritative_ledger_projection_suppresses_stale_live_child_tokens() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, _rx) = feed_with_rx();
    feed.supports_sync = true;
    app.subagents.feeds.insert("worker".into(), feed);
    app.update_subagent_bar(vec![subagent("worker", "running")]);
    app.select_agent(Some("worker"));
    app.ensure_session("worker");
    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 1,
            "rev": 1,
            "messages": [{"id":"a1","role":"assistant","content":"from ledger"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    app.route_subagent_event(
        "worker",
        Event::Token {
            token: "legacy duplicate".into(),
        },
    );

    let entries = app.subagents.sessions["worker"].chat.entries();
    assert!(
        matches!(entries, [ChatEntry::Assistant { text, streaming: false }] if text == "from ledger"),
        "focused authoritative ledger projection must suppress stale live tokens after caught-up sync: {entries:?}"
    );
}

#[tokio::test]
async fn focused_authoritative_child_turn_end_finalizes_after_focus_switch() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, _rx) = feed_with_rx();
    feed.supports_sync = true;
    app.subagents.feeds.insert("worker".into(), feed);
    app.update_subagent_bar(vec![subagent("worker", "running")]);
    app.select_agent(Some("worker"));
    app.ensure_session("worker");
    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 1,
            "rev": 1,
            "messages": [{"id":"u1","role":"user","content":"initial task"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );
    app.route_subagent_event("worker", Event::TurnStart);
    app.route_subagent_event(
        "worker",
        Event::Token {
            token: "live work".into(),
        },
    );

    app.select_agent(None);
    app.route_subagent_event("worker", Event::TurnEnd { message: json!({}) });

    let entries = app.subagents.sessions["worker"].chat.entries();
    assert!(
        matches!(entries, [ChatEntry::User { text, .. }, ChatEntry::Assistant { text: live, streaming: false }] if text == "initial task" && live == "live work"),
        "turn end must finalize existing focused live output even if focus moved away before ledger reconciliation: {entries:?}"
    );
}

#[tokio::test]
async fn focused_authoritative_child_renders_live_tokens_until_ledger_reconciles() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, _rx) = feed_with_rx();
    feed.supports_sync = true;
    app.subagents.feeds.insert("worker".into(), feed);
    app.update_subagent_bar(vec![subagent("worker", "running")]);
    app.select_agent(Some("worker"));
    app.ensure_session("worker");
    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 1,
            "rev": 1,
            "messages": [{"id":"u1","role":"user","content":"initial task"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    app.route_subagent_event("worker", Event::TurnStart);
    app.route_subagent_event(
        "worker",
        Event::Token {
            token: "live work".into(),
        },
    );

    let entries = app.subagents.sessions["worker"].chat.entries();
    assert!(
        matches!(entries, [ChatEntry::User { text, .. }, ChatEntry::Assistant { text: live, streaming: true }] if text == "initial task" && live == "live work"),
        "focused busy child must render live output before turn commit: {entries:?}"
    );

    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 1,
            "rev": 2,
            "messages": [{"id":"a1","role":"assistant","content":"committed work"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    let entries = app.subagents.sessions["worker"].chat.entries();
    assert!(
        matches!(entries, [ChatEntry::User { text, .. }, ChatEntry::Assistant { text: committed, streaming: false }] if text == "initial task" && committed == "committed work"),
        "ledger reconciliation must replace the focused live projection without duplication: {entries:?}"
    );
}
