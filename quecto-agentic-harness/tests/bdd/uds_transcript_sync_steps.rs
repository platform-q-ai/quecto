//! Step definitions for `uds_transcript_sync.feature` (D3 #1973, #1857):
//! the idle-loop `sync` command driven over the REAL UDS server of a
//! seeded persisted session (the plumbing of `uds_paged_history_steps`,
//! seeded like `uds_history_recovery_steps`): a delta after the client's
//! revision, the empty caught-up delta, the resync page for another epoch
//! or a revision before the first commit, and the parse-error and
//! child-addressed refusals. The busy reader fast path is covered by
//! `uds_subagent_liveness.feature`.

use super::uds_history_recovery_steps::{attach, response, seed_session};
use super::uds_paged_history_steps::{wait_for_paged_event, write_command};
use super::*;
use quecto::domain::message::Message;
use std::time::Duration;

/// Send `cmd` and record the first response the predicate accepts.
fn send_and_record(
    world: &mut QuectoWorld,
    cmd: serde_json::Value,
    accept: impl Fn(&serde_json::Value) -> bool + 'static,
) {
    write_command(world, 1, &cmd);
    let response = wait_for_paged_event(
        world,
        1,
        Duration::from_secs(5),
        "the sync response",
        move |event| {
            event.get("type").and_then(|t| t.as_str()) == Some("response") && accept(event)
        },
    );
    world._paged_response = Some(response);
}

fn send_sync(world: &mut QuectoWorld, id: &str, cmd: serde_json::Value) {
    let expected = id.to_string();
    send_and_record(world, cmd, move |event| {
        event.get("command").and_then(|c| c.as_str()) == Some("sync")
            && event.get("id").and_then(|i| i.as_str()) == Some(expected.as_str())
    });
}

fn seeded_contents(world: &QuectoWorld) -> Vec<String> {
    world._paged_seeded.clone()
}

// ── Given ───────────────────────────────────────────────────────────────────

#[given("a persisted UDS session containing three plain messages")]
fn given_three_messages(world: &mut QuectoWorld) {
    let messages = vec![
        Message::user("first question"),
        Message::assistant("first answer", vec![]),
        Message::user("second question"),
    ];
    world._paged_seeded = messages.iter().map(|m| m.content.clone()).collect();
    seed_session(world, messages);
}

// ── When ────────────────────────────────────────────────────────────────────

#[when(expr = "a client syncs the current epoch from revision {int}")]
fn when_sync_current_epoch(world: &mut QuectoWorld, since_rev: u64) {
    let attached = attach(world);
    assert_eq!(attached["hasMoreBefore"], false, "{attached}");
    send_sync(
        world,
        "sync-current",
        serde_json::json!({"type": "sync", "id": "sync-current", "epoch": 0, "sinceRev": since_rev}),
    );
}

#[when(expr = "a client syncs epoch {int} from revision {int}")]
fn when_sync_epoch(world: &mut QuectoWorld, epoch: u64, since_rev: u64) {
    attach(world);
    send_sync(
        world,
        "sync-epoch",
        serde_json::json!({"type": "sync", "id": "sync-epoch", "epoch": epoch, "sinceRev": since_rev}),
    );
}

#[when(expr = "a client sends a sync whose revision is the string {string}")]
fn when_sync_string_revision(world: &mut QuectoWorld, since_rev: String) {
    attach(world);
    send_and_record(
        world,
        serde_json::json!({"type": "sync", "id": "sync-bad", "epoch": 0, "sinceRev": since_rev}),
        |event| event.get("command").and_then(|c| c.as_str()) == Some("parse_error"),
    );
}

#[when(expr = "an idle client sends a sync addressed to child {string}")]
fn when_sync_addressed_to_child(world: &mut QuectoWorld, child: String) {
    attach(world);
    send_sync(
        world,
        "sync-child",
        serde_json::json!({"type": "sync", "id": "sync-child", "agent_id": child, "epoch": 0, "sinceRev": 0}),
    );
}

// ── Then ────────────────────────────────────────────────────────────────────

#[then(
    expr = "the sync response should carry the {int} newest messages in order, caught up at revision {int}"
)]
fn then_delta(world: &mut QuectoWorld, count: usize, rev: u64) {
    let seeded = seeded_contents(world);
    let response = response(world);
    assert_eq!(response["success"], true, "{response}");
    let data = &response["data"];
    assert_eq!(data["resync"], false, "{data}");
    assert_eq!(data["caughtUp"], true, "{data}");
    assert!(data["nextRev"].is_null(), "{data}");
    assert_eq!(data["epoch"], 0, "{data}");
    assert_eq!(data["rev"], rev, "{data}");
    assert!(
        data.get("hasMoreBefore").is_none(),
        "a delta is not a page: {data}"
    );
    let contents: Vec<String> = data["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .map(|m| m["content"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(contents, seeded[seeded.len() - count..].to_vec(), "{data}");
}

#[then(expr = "the sync response should be a resync page of {int} messages at revision {int}")]
fn then_resync(world: &mut QuectoWorld, count: usize, rev: u64) {
    let seeded = seeded_contents(world);
    let response = response(world);
    assert_eq!(response["success"], true, "{response}");
    let data = &response["data"];
    assert_eq!(data["resync"], true, "{data}");
    assert_eq!(data["caughtUp"], true, "{data}");
    assert!(data["nextRev"].is_null(), "{data}");
    assert_eq!(data["epoch"], 0, "{data}");
    assert_eq!(data["rev"], rev, "{data}");
    assert_eq!(data["hasMoreBefore"], false, "{data}");
    assert!(data["before"].is_null(), "{data}");
    let contents: Vec<String> = data["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .map(|m| m["content"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(contents.len(), count, "{data}");
    assert_eq!(contents, seeded, "{data}");
}
