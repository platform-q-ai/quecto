//! Step definitions for `tui_end_of_turn_refs.feature` (#1060 / ADR-0008 part 2).
//!
//! Drives the real App recovery path via `TuiHarness` the same way the
//! `app_events_1060_*` unit tests do: stream events, end-of-turn refs,
//! fetch-on-miss, request-id gated recovery responses. Child scenarios use the
//! connect-on-select child stream (`route`), which is the production path that
//! applies end-of-turn refs for sub-agents (parent-forwarded
//! `subagent_messages_appended` is superseded by that stream).

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::infrastructure::client::Event;
use quecto_tui::interface::app::tui_harness::{
    TuiHarness, spawn_start, spawn_subagent_socket_with_commands, subagent_with_socket,
    subagents_changed,
};

/// Stable refs used across master recovery scenarios.
const TEXT_REF: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
const TOOL_CALL_REF: &str = "bbbbbbbb-bbbb-cccc-dddd-eeeeeeeeeeee";
const TOOL_RESULT_REF: &str = "cccccccc-cccc-dddd-eeee-ffffffffffff";
const FINAL_TEXT_REF: &str = "dddddddd-dddd-eeee-ffff-000000000000";
const CHILD_REF: &str = "eeeeeeee-eeee-ffff-0000-111111111111";

const RECOVERED_ASSISTANT: &str = "FULL_RECOVERED_ASSISTANT_BODY";
const RECOVERED_TOOL_RESULT: &str = "tool-result-body";
const RECOVERED_FINAL_TEXT: &str = "final-text-after-tool";
const RECOVERED_CHILD: &str = "FULL_CHILD_BODY";
const WRONG_RECOVERY_BODY: &str = "SHOULD_NOT_APPEAR";

fn init_fresh(world: &mut TuiWorld) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let h = rt.block_on(TuiHarness::new());
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
    world.tui_last_commands.clear();
    world.tui_viewed_agent = None;
    world.tui_subagent_commands = None;
}

fn drive<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let _guard = handle.enter();
    let h = &mut world.tui_parity.as_mut().expect("harness").0;
    f(h)
}

fn drain_commands(world: &mut TuiWorld) -> Vec<String> {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let h = &mut world.tui_parity.as_mut().expect("harness").0;
    handle.block_on(h.drain_commands())
}

fn try_drain(world: &mut TuiWorld) -> Vec<String> {
    drive(world, |h| h.try_drain_commands())
}

fn json_field(line: &str, field: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    value.get(field)?.as_str().map(str::to_string)
}

fn is_get_message(line: &str) -> bool {
    json_field(line, "type").as_deref() == Some("get_message")
}

fn get_message_cmds(commands: &[String]) -> Vec<&String> {
    commands.iter().filter(|l| is_get_message(l)).collect()
}

fn get_message_ids(commands: &[String]) -> Vec<String> {
    commands
        .iter()
        .filter(|l| is_get_message(l))
        .filter_map(|l| json_field(l, "id"))
        .collect()
}

fn message_id_of(line: &str) -> Option<String> {
    json_field(line, "messageId").or_else(|| json_field(line, "message_id"))
}

fn agent_id_of(line: &str) -> Option<String> {
    json_field(line, "agent_id").or_else(|| json_field(line, "agentId"))
}

fn store_commands(world: &mut TuiWorld, extra: Vec<String>) {
    let mut cmds = world.tui_last_commands.clone();
    cmds.extend(extra);
    world.tui_last_commands = cmds;
}

fn store_try_drain(world: &mut TuiWorld) {
    let extra = try_drain(world);
    store_commands(world, extra);
}

fn store_drain(world: &mut TuiWorld) {
    let extra = drain_commands(world);
    store_commands(world, extra);
}

fn turn_end_text_refs(refs: &[&str]) -> Event {
    Event::TurnEnd {
        message: serde_json::json!({
            "role": "assistant",
            "content": "",
            "messageRefs": refs,
        }),
    }
}

fn turn_end_with_context(refs: &[&str], used: u64, max: u64) -> Event {
    Event::TurnEnd {
        message: serde_json::json!({
            "role": "assistant",
            "content": "",
            "messageRefs": refs,
            "contextTokens": used,
            "maxContextTokens": max,
        }),
    }
}

fn turn_end_miss(refs: &[&str], content_len: u64) -> Event {
    Event::TurnEnd {
        message: serde_json::json!({
            "role": "assistant",
            "content": "",
            "messageRefs": refs,
            "contentLength": content_len,
        }),
    }
}

// ── Given ───────────────────────────────────────────────────────────────────

#[given("a fresh TUI app harness connected for the active turn")]
fn given_fresh_connected(world: &mut TuiWorld) {
    init_fresh(world);
    drive(world, |h| {
        h.event(Event::AgentStart);
    });
    world.tui_last_commands = try_drain(world);
}

#[given(expr = "the assistant has streamed tokens {string} then {string}")]
fn given_streamed_two_tokens(world: &mut TuiWorld, a: String, b: String) {
    drive(world, |h| {
        h.event(Event::Token { token: a });
        h.event(Event::Token { token: b });
    });
}

#[given(expr = "the assistant has streamed tokens {string}")]
fn given_streamed_tokens(world: &mut TuiWorld, token: String) {
    drive(world, |h| {
        h.event(Event::Token { token });
    });
}

#[given(expr = "the assistant has streamed a tool call for {string} with result {string}")]
fn given_streamed_tool(world: &mut TuiWorld, tool: String, result: String) {
    drive(world, |h| {
        h.event(Event::ToolExecutionStart {
            tool_call_id: "c1".into(),
            tool_name: tool.clone(),
            args: serde_json::json!({}),
        });
        h.event(Event::ToolExecutionEnd {
            tool_call_id: "c1".into(),
            tool_name: tool,
            result: serde_json::json!({"content":[{"type":"text","text": result}]}),
            is_error: false,
        });
    });
}

#[given("a TUI that connected mid-turn and missed early tokens of the active turn")]
fn given_mid_turn_miss_text(world: &mut TuiWorld) {
    init_fresh(world);
    drive(world, |h| {
        h.event(Event::AgentStart);
        // Placeholder only — proves the client holds incomplete content.
        h.event(Event::Token {
            token: "…".into()
        });
    });
    world.tui_last_commands = try_drain(world);
}

#[given("a TUI that connected mid-turn and missed tool_execution events of the active turn")]
fn given_mid_turn_miss_tools(world: &mut TuiWorld) {
    init_fresh(world);
    drive(world, |h| {
        // Agent is running; no tool_execution_* events were observed.
        h.event(Event::AgentStart);
    });
    world.tui_last_commands = try_drain(world);
}

#[given("a turn_end has arrived that identifies the turn messages by non-empty refs")]
fn given_turn_end_already_arrived(world: &mut TuiWorld) {
    drive(world, |h| {
        h.event(turn_end_miss(&[TEXT_REF], 64));
    });
    store_drain(world);
}

#[given("the TUI has outstanding recovery requests for those refs")]
fn given_outstanding_recovery(world: &mut TuiWorld) {
    let fetches = get_message_cmds(&world.tui_last_commands);
    assert!(
        !fetches.is_empty(),
        "expected outstanding get_message recovery requests, got {:?}",
        world.tui_last_commands
    );
}

#[given(expr = "sub-agent {string} has already streamed {string} for the active child turn")]
fn given_child_streamed(world: &mut TuiWorld, id: String, token: String) {
    drive(world, |h| {
        h.route(&id, Event::AgentStart);
        h.route(&id, Event::Token { token });
    });
    // Drop connect-on-select noise so later fetch assertions stay clean.
    world.tui_last_commands = try_drain(world);
}

#[given(expr = "a TUI viewing sub-agent {string} that connected mid-turn")]
fn given_viewing_child_mid_turn(world: &mut TuiWorld, id: String) {
    // Reuse the production connect-on-select harness, then leave only a partial
    // live token so end-of-turn recovery must fetch.
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let (mut h, cmd_rx) = rt.block_on(async {
        let mut h = TuiHarness::new().await;
        h.event(Event::AgentStart);
        h.event(spawn_start(&id));
        let (socket, cmd_rx) = spawn_subagent_socket_with_commands(&id);
        h.event(subagents_changed(vec![subagent_with_socket(
            &id,
            "running",
            Some(("active", 0, 3)),
            Some(socket),
        )]));
        h.select(Some(&id));
        h.route(&id, Event::AgentStart);
        h.route(
            &id,
            Event::Token {
                token: "…".into()
            },
        );
        (h, cmd_rx)
    });
    h.try_drain_commands();
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
    world.tui_subagent_commands = Some(cmd_rx);
    world.tui_viewed_agent = Some(id);
    world.tui_last_commands.clear();
}

// ── When ────────────────────────────────────────────────────────────────────

#[when("a turn_end arrives that identifies the assistant message by non-empty refs only")]
fn when_turn_end_text_refs(world: &mut TuiWorld) {
    drive(world, |h| {
        h.event(turn_end_text_refs(&[TEXT_REF]));
    });
    store_try_drain(world);
}

#[when("a turn_end arrives that identifies the tool and text messages by non-empty refs only")]
fn when_turn_end_tool_text_refs(world: &mut TuiWorld) {
    drive(world, |h| {
        h.event(turn_end_text_refs(&[
            TOOL_CALL_REF,
            TOOL_RESULT_REF,
            FINAL_TEXT_REF,
        ]));
    });
    store_try_drain(world);
}

#[when(
    expr = "a turn_end arrives with contextTokens {int} and maxContextTokens {int} and non-empty message refs"
)]
fn when_turn_end_with_context(world: &mut TuiWorld, used: u64, max: u64) {
    drive(world, |h| {
        h.event(turn_end_with_context(&[TEXT_REF], used, max));
    });
    store_try_drain(world);
}

#[when("a turn_end arrives that identifies the turn messages by non-empty refs")]
fn when_turn_end_miss_text(world: &mut TuiWorld) {
    drive(world, |h| {
        h.event(turn_end_miss(&[TEXT_REF], 64));
    });
    store_drain(world);
}

#[when(
    "a turn_end arrives that identifies assistant tool-call and tool-result messages by non-empty refs"
)]
fn when_turn_end_miss_tools(world: &mut TuiWorld) {
    drive(world, |h| {
        h.event(turn_end_text_refs(&[
            TOOL_CALL_REF,
            TOOL_RESULT_REF,
            FINAL_TEXT_REF,
        ]));
    });
    store_drain(world);
}

#[when("the matching recovery responses arrive for those requests")]
fn when_recovery_responses_arrive(world: &mut TuiWorld) {
    let fetches = get_message_cmds(&world.tui_last_commands);
    assert!(
        !fetches.is_empty(),
        "expected get_message recovery requests before delivering responses: {:?}",
        world.tui_last_commands
    );

    // Map messageId → request id so responses match whatever order was issued.
    let mut by_mid: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for line in &fetches {
        let mid = message_id_of(line).unwrap_or_default();
        let rid = json_field(line, "id").unwrap_or_default();
        assert!(
            !mid.is_empty() && !rid.is_empty(),
            "get_message must carry id+messageId: {line}"
        );
        by_mid.insert(mid, rid);
    }

    // Child recovery path (single assistant ref).
    if by_mid.contains_key(CHILD_REF) {
        let rid = by_mid.get(CHILD_REF).unwrap().clone();
        drive(world, |h| {
            h.event(Event::Response {
                id: Some(rid),
                command: "get_message".into(),
                success: true,
                data: Some(serde_json::json!({
                    "id": CHILD_REF,
                    "role": "assistant",
                    "content": RECOVERED_CHILD,
                })),
                error: None,
            });
        });
        return;
    }

    // Master text-only recovery.
    if by_mid.contains_key(TEXT_REF) && by_mid.len() == 1 {
        let rid = by_mid.get(TEXT_REF).unwrap().clone();
        drive(world, |h| {
            h.event(Event::Response {
                id: Some(rid),
                command: "get_message".into(),
                success: true,
                data: Some(serde_json::json!({
                    "id": TEXT_REF,
                    "role": "assistant",
                    "content": RECOVERED_ASSISTANT,
                })),
                error: None,
            });
        });
        return;
    }

    // Master multi-role recovery (tool-call + tool-result + final text).
    let payloads = [
        (
            TOOL_CALL_REF,
            serde_json::json!({
                "id": TOOL_CALL_REF,
                "role": "assistant",
                "content": "",
                "toolCalls": [{"id":"c1","name":"bash","arguments":"{}"}]
            }),
        ),
        (
            TOOL_RESULT_REF,
            serde_json::json!({
                "id": TOOL_RESULT_REF,
                "role": "tool",
                "content": RECOVERED_TOOL_RESULT,
                "toolCallId": "c1",
                "toolName": "bash"
            }),
        ),
        (
            FINAL_TEXT_REF,
            serde_json::json!({
                "id": FINAL_TEXT_REF,
                "role": "assistant",
                "content": RECOVERED_FINAL_TEXT
            }),
        ),
    ];
    drive(world, |h| {
        for (mid, data) in payloads {
            let rid = by_mid
                .get(mid)
                .unwrap_or_else(|| panic!("missing get_message for ref {mid}; map={by_mid:?}"))
                .clone();
            h.event(Event::Response {
                id: Some(rid),
                command: "get_message".into(),
                success: true,
                data: Some(data),
                error: None,
            });
        }
    });
}

#[when("a get_message recovery response arrives with a non-matching request id")]
fn when_mismatched_recovery(world: &mut TuiWorld) {
    drive(world, |h| {
        h.event(Event::Response {
            id: Some("not-our-recovery-id".into()),
            command: "get_message".into(),
            success: true,
            data: Some(serde_json::json!({
                "id": TEXT_REF,
                "role": "assistant",
                "content": WRONG_RECOVERY_BODY,
            })),
            error: None,
        });
    });
}

#[when("a child turn_end arrives identifying those messages by refs only")]
fn when_child_end_streamed_refs(world: &mut TuiWorld) {
    // Production applies child end-of-turn refs on the child's own stream
    // (TurnEnd / AgentEnd). Parent-forwarded subagent_messages_appended is
    // ignored once connect-on-select is active; drive the live child path.
    let id = world
        .tui_viewed_agent
        .clone()
        .expect("viewing sub-agent given must set tui_viewed_agent");
    drive(world, |h| {
        h.route(
            &id,
            Event::TurnEnd {
                message: serde_json::json!({
                    "role": "assistant",
                    "content": "",
                    "messageRefs": [CHILD_REF],
                }),
            },
        );
    });
    store_try_drain(world);
}

#[when("a child turn_end arrives identifying the child messages by non-empty refs")]
fn when_child_end_miss_refs(world: &mut TuiWorld) {
    let id = world
        .tui_viewed_agent
        .clone()
        .expect("viewing sub-agent given must set tui_viewed_agent");
    drive(world, |h| {
        h.route(
            &id,
            Event::TurnEnd {
                message: serde_json::json!({
                    "role": "assistant",
                    "content": "",
                    "messageRefs": [CHILD_REF],
                    "contentLength": 64,
                }),
            },
        );
    });
    store_drain(world);
}

// ── Then ────────────────────────────────────────────────────────────────────
//
// Shared steps reused from other modules (do not redefine):
//   - "the app master session shows {string}"
//   - "the app master session shows {string} exactly once"
//   - "the sub-agent's session shows {string}"
//   - "a TUI viewing sub-agent {string}"

#[then(expr = "the app master session shows the tool call {string}")]
fn then_master_shows_tool(world: &mut TuiWorld, tool: String) {
    let entries = drive(world, |h| h.master_tool_entries());
    assert!(
        entries.iter().any(|(name, _)| name == &tool),
        "master session should contain tool call {tool:?}; entries={entries:?}"
    );
}

#[then("the app master session shows the full assistant content for the active turn")]
fn then_master_shows_recovered(world: &mut TuiWorld) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains(RECOVERED_ASSISTANT),
        "master session should show recovered assistant body, got:\n{frame}"
    );
}

#[then("the app master session shows that content exactly once")]
fn then_master_recovered_once(world: &mut TuiWorld) {
    let frame = drive(world, |h| h.full_frame());
    let needle = if frame.contains(RECOVERED_ASSISTANT) {
        RECOVERED_ASSISTANT
    } else if frame.contains(RECOVERED_FINAL_TEXT) {
        RECOVERED_FINAL_TEXT
    } else {
        panic!("expected recovered content in master session, got:\n{frame}");
    };
    assert_eq!(
        frame.matches(needle).count(),
        1,
        "recovered content must not duplicate:\n{frame}"
    );
}

#[then("the app master session shows the tool call and tool result in order")]
fn then_master_tool_order(world: &mut TuiWorld) {
    let entries = drive(world, |h| h.master_tool_entries());
    assert_eq!(
        entries.len(),
        1,
        "expected one recovered tool entry: {entries:?}"
    );
    assert_eq!(entries[0].0, "bash", "recovered tool identity/order");
    assert_eq!(
        entries[0].1.as_deref(),
        Some(RECOVERED_TOOL_RESULT),
        "result must attach to the preceding bash call"
    );
}

#[then("the app master session shows the final assistant text for the active turn exactly once")]
fn then_master_final_text_once(world: &mut TuiWorld) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains(RECOVERED_FINAL_TEXT),
        "master session should show final recovered text, got:\n{frame}"
    );
    assert_eq!(
        frame.matches(RECOVERED_FINAL_TEXT).count(),
        1,
        "final text must appear exactly once:\n{frame}"
    );
}

#[then("the app master session does not apply that recovery payload")]
fn then_master_ignores_wrong_recovery(world: &mut TuiWorld) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        !frame.contains(WRONG_RECOVERY_BODY),
        "mismatched recovery id must not alter the transcript:\n{frame}"
    );
}

#[then("the TUI still awaits the recovery response for its own request id")]
fn then_still_awaits_own_recovery(world: &mut TuiWorld) {
    let ids = get_message_ids(&world.tui_last_commands);
    assert!(
        !ids.is_empty(),
        "TUI should still hold its own recovery request ids: {:?}",
        world.tui_last_commands
    );
    // Delivering the correct response after the wrong one must still reconcile.
    let rid = ids[0].clone();
    drive(world, |h| {
        h.event(Event::Response {
            id: Some(rid),
            command: "get_message".into(),
            success: true,
            data: Some(serde_json::json!({
                "id": TEXT_REF,
                "role": "assistant",
                "content": RECOVERED_ASSISTANT,
            })),
            error: None,
        });
    });
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains(RECOVERED_ASSISTANT),
        "own recovery response must still apply after a mismatched id was ignored:\n{frame}"
    );
}

#[then("the TUI issues no on-demand message fetches for the completed turn")]
fn then_no_fetches_master(world: &mut TuiWorld) {
    let mut cmds = world.tui_last_commands.clone();
    cmds.extend(try_drain(world));
    world.tui_last_commands = cmds;
    let fetches = get_message_cmds(&world.tui_last_commands);
    assert!(
        fetches.is_empty(),
        "common streamed path must issue ZERO get_message fetches; got: {:?}",
        world.tui_last_commands
    );
}

#[then("the TUI issues no on-demand message fetches for the completed child turn")]
fn then_no_fetches_child(world: &mut TuiWorld) {
    let fetches = get_message_ids(&world.tui_last_commands);
    assert!(
        fetches.is_empty(),
        "completed streamed child turn must issue no get_message fetches; got: {:?}",
        world.tui_last_commands
    );
}

#[then("the footer reflects context usage from the turn_end metadata")]
fn then_footer_context(world: &mut TuiWorld) {
    let footer = drive(world, |h| h.master_footer_text());
    // Matches unit test / existing footer formatting (40_000/200_000 → 40k/200k).
    assert!(
        footer.contains("40k/200k") || (footer.contains("40k") && footer.contains("200k")),
        "footer must use turn_end context metadata, got: {footer}"
    );
    let fetches = get_message_cmds(&world.tui_last_commands);
    assert!(
        fetches.is_empty(),
        "footer path must not fetch messages: {:?}",
        world.tui_last_commands
    );
}

#[then("the TUI requests only the missing message content for those refs")]
fn then_fetches_missing_master(world: &mut TuiWorld) {
    let fetches = get_message_cmds(&world.tui_last_commands);
    assert!(
        !fetches.is_empty(),
        "mid-turn miss must issue get_message for missing refs; cmds={:?}",
        world.tui_last_commands
    );
    let requested: Vec<String> = fetches
        .iter()
        .map(|line| message_id_of(line).expect("get_message must carry messageId"))
        .collect();
    let expected: Vec<String> = if requested.iter().any(|id| id == TOOL_CALL_REF) {
        vec![
            TOOL_CALL_REF.into(),
            TOOL_RESULT_REF.into(),
            FINAL_TEXT_REF.into(),
        ]
    } else {
        vec![TEXT_REF.into()]
    };
    assert_eq!(
        requested, expected,
        "must fetch exactly the missing refs in order"
    );
    let ids = get_message_ids(&world.tui_last_commands);
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(ids.len(), expected.len(), "one request per missing ref");
    assert_eq!(
        unique.len(),
        ids.len(),
        "recovery request ids must be unique"
    );
    assert!(
        ids.iter().all(|id| !id.is_empty()),
        "request ids must be non-empty"
    );
}

#[then("the TUI requests only the missing child message content for those refs")]
fn then_fetches_missing_child(world: &mut TuiWorld) {
    let fetches = get_message_cmds(&world.tui_last_commands);
    assert!(
        !fetches.is_empty(),
        "child mid-turn miss must issue recovery get_message: {:?}",
        world.tui_last_commands
    );
    let id = world
        .tui_viewed_agent
        .as_deref()
        .expect("viewed sub-agent id");
    assert!(
        fetches.iter().all(|l| {
            agent_id_of(l).as_deref() == Some(id) && message_id_of(l).as_deref() == Some(CHILD_REF)
        }),
        "recovery must target the ending child by agent_id via master: {fetches:?}"
    );
}

#[then(expr = "the sub-agent's session shows {string} exactly once")]
fn then_child_shows_once(world: &mut TuiWorld, expected: String) {
    let frame = drive(world, |h| h.full_frame());
    let count = frame.matches(&expected).count();
    assert_eq!(
        count, 1,
        "sub-agent session should show {expected:?} exactly once (got {count}):\n{frame}"
    );
}

#[then("the sub-agent's session shows the full child turn content")]
fn then_child_shows_recovered(world: &mut TuiWorld) {
    let frame = drive(world, |h| h.full_frame());
    assert!(
        frame.contains(RECOVERED_CHILD),
        "sub-agent session should show recovered child body, got:\n{frame}"
    );
}

#[then("the sub-agent's session shows that content exactly once")]
fn then_child_recovered_once(world: &mut TuiWorld) {
    let frame = drive(world, |h| h.full_frame());
    assert_eq!(
        frame.matches(RECOVERED_CHILD).count(),
        1,
        "recovered child content must not duplicate:\n{frame}"
    );
}
