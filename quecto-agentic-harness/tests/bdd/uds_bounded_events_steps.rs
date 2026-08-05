//! #1060 / ADR-0008 part 2 — BDD steps for `uds_bounded_events.feature`.
//!
//! End-of-turn events identify messages by stable `messageRefs` instead of
//! re-carrying full content. These steps assert size bounds, ref presence,
//! on-demand `get_message` recovery, busy-connect snapshot id parity, and the
//! re-stamped sub-agent path.

use super::*;
use quecto::interface::cli::protocol::EVENT_LINE_CAP_BYTES;
use quecto_line_io::PROTOCOL_FRAME_CAP_BYTES;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Budget for "well under the frame size limit" (matches unit tests).
const WELL_UNDER_FRAME: usize = EVENT_LINE_CAP_BYTES / 4;

fn parse_events(lines: &[String]) -> Vec<serde_json::Value> {
    lines
        .iter()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn events_of_type<'a>(events: &'a [serde_json::Value], ty: &str) -> Vec<&'a serde_json::Value> {
    events
        .iter()
        .filter(|e| e.get("type").and_then(|t| t.as_str()) == Some(ty))
        .collect()
}

fn non_empty_refs(v: &serde_json::Value) -> Vec<String> {
    let candidates = [
        v.get("messageRefs"),
        v.get("message").and_then(|m| m.get("messageRefs")),
        v.get("message_refs"),
        v.get("message").and_then(|m| m.get("message_refs")),
    ];
    for c in candidates.into_iter().flatten() {
        if let Some(arr) = c.as_array() {
            let refs: Vec<String> = arr
                .iter()
                .filter_map(|item| {
                    if let Some(s) = item.as_str() {
                        return Some(s.to_string());
                    }
                    item.get("id")
                        .and_then(|id| id.as_str())
                        .map(str::to_string)
                })
                .filter(|s| !s.is_empty())
                .collect();
            if !refs.is_empty() {
                return refs;
            }
        }
    }
    Vec::new()
}

fn event_line_len(raw_lines: &[String], ty: &str) -> Option<usize> {
    raw_lines.iter().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        (v.get("type").and_then(|t| t.as_str()) == Some(ty)).then_some(l.len())
    })
}

fn issue_1094_body() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    (0..EVENT_LINE_CAP_BYTES + 1)
        .map(|idx| ALPHABET[idx % ALPHABET.len()] as char)
        .collect()
}

fn seed_oversized_message(world: &mut QuectoWorld) {
    let content = issue_1094_body();
    mount_sequential_text_responses(world, &[(&content, Duration::ZERO)]);
    world._bounded_expected_body = Some(content);
    world.mc_mode = true;
    world.no_session = true;
    world._mc_persist = true;
    if !world.mc_connected_clients.contains(&1) {
        world.mc_connected_clients.push(1);
    }
    world
        .mc_client_commands
        .entry(1)
        .or_default()
        .push(serde_json::json!({"type":"prompt","message":"seed oversized prior"}).to_string());

    drive_mc_first_turn_keep_alive(world);
    let refs = completed_turn_refs(world, 1);
    assert_eq!(
        refs.len(),
        1,
        "oversized seed should create exactly one assistant message ref; got {refs:?}"
    );
    world._bounded_recorded_ref = refs.first().cloned();
    world._bounded_message_refs = refs;
}

fn completed_turn_refs(world: &QuectoWorld, client_id: u32) -> Vec<String> {
    let events = world
        .mc_client_events
        .get(&client_id)
        .cloned()
        .unwrap_or_default();
    let parsed = parse_events(&events);
    let mut refs = Vec::new();
    for ty in ["turn_end", "agent_end"] {
        for event in events_of_type(&parsed, ty) {
            refs.extend(non_empty_refs(event));
        }
    }
    refs.sort();
    refs.dedup();
    assert!(
        !refs.is_empty(),
        "expected completed turn to emit message refs; events: {events:#?}"
    );
    refs
}

fn read_matching_response(
    world: &mut QuectoWorld,
    client_id: u32,
    request_id: &str,
    timeout: Duration,
) -> serde_json::Value {
    let deadline = Instant::now() + timeout;
    // Parse only newly arrived lines. The paged oversized-message reads call
    // this once per page, so re-parsing the whole accumulated buffer each poll
    // is quadratic in the number of pages times their (multi-megabyte) size.
    let mut scanned = 0usize;
    loop {
        drain_client_events(world, client_id, Duration::from_millis(100));
        if let Some(events) = world.mc_client_events.get(&client_id) {
            let found = events.iter().skip(scanned).find_map(|line| {
                let value: serde_json::Value = serde_json::from_str(line).ok()?;
                (value.get("type").and_then(|v| v.as_str()) == Some("response")
                    && value.get("id").and_then(|v| v.as_str()) == Some(request_id))
                .then_some(value)
            });
            if let Some(response) = found {
                return response;
            }
            scanned = events.len();
        }
        if Instant::now() > deadline {
            let events = world
                .mc_client_events
                .get(&client_id)
                .cloned()
                .unwrap_or_default();
            panic!("timeout waiting for response id={request_id}; events: {events:#?}");
        }
    }
}

fn collect_paged_oversized_response(world: &mut QuectoWorld) {
    let client_id = if world._mc_live_busy { 2 } else { 1 };
    world._bounded_oversized_client_id = Some(client_id);
    if !world._mc_live_streams.contains_key(&client_id) {
        connect_client_live(world, client_id);
    }
    let mid = world
        ._bounded_recorded_ref
        .clone()
        .expect("oversized message ref not seeded");
    world._bounded_get_message_responses.clear();
    let mut offset = 0usize;
    for idx in 0..16 {
        let request_id = format!("oversized-page-{idx}");
        let cmd = serde_json::json!({
            "type": "get_message",
            "id": request_id,
            "messageId": mid,
            "offset": offset,
            "limit": EVENT_LINE_CAP_BYTES / 2,
        });
        let stream = world
            ._mc_live_streams
            .get_mut(&client_id)
            .expect("live UDS client stream for oversized get_message");
        writeln!(stream, "{cmd}").expect("write get_message command");
        stream.flush().expect("flush get_message command");

        let response =
            read_matching_response(world, client_id, &request_id, Duration::from_secs(5));
        assert_eq!(
            response.get("success").and_then(|v| v.as_bool()),
            Some(true),
            "paged get_message should succeed: {response}"
        );
        let data = response.get("data").expect("get_message response data");
        assert_eq!(data.get("id").and_then(|v| v.as_str()), Some(mid.as_str()));
        let next = data
            .get("nextOffset")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        let has_more = data
            .get("hasMoreContent")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        world._bounded_get_message_responses.push(response);
        if !has_more {
            break;
        }
        let Some(next) = next else {
            panic!("paged get_message response with hasMoreContent lacked nextOffset");
        };
        assert!(next > offset, "paged get_message must make progress");
        offset = next;
    }
}

fn reassembled_oversized_content(world: &QuectoWorld) -> String {
    world
        ._bounded_get_message_responses
        .iter()
        .map(|resp| {
            resp.get("data")
                .and_then(|d| d.get("content"))
                .and_then(|c| c.as_str())
                .unwrap_or("")
        })
        .collect()
}

fn content_re_carried(v: &serde_json::Value) -> bool {
    // turn_end: message.content must be empty (or absent).
    if v.get("type").and_then(|t| t.as_str()) == Some("turn_end") {
        let c = v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("");
        return !c.is_empty();
    }
    // agent_end / subagent_messages_appended: messages array must not re-carry
    // substantial content.
    if let Some(msgs) = v.get("messages").and_then(|m| m.as_array()) {
        for m in msgs {
            let c = m.get("content").and_then(|c| c.as_str()).unwrap_or("");
            if c.len() > 64 {
                return true;
            }
            if !c.is_empty() && non_empty_refs(v).is_empty() {
                // Legacy full carry without refs.
                return true;
            }
            if !c.is_empty() {
                // Any non-empty body on the wire when refs are present is re-carry.
                return true;
            }
        }
    }
    false
}

fn ensure_uds_events(world: &mut QuectoWorld) {
    if world.mc_mode {
        if world.mc_exit_code.is_none() {
            // Prefer the live multi-client driver when the scenario requested it.
            if world._mc_live_busy {
                drive_mc_live_busy(world);
            } else {
                // Fall back to the batch multi-client path.
                // execute_multi_client_uds is private; close-all is the public entry.
                // Call via when_close path by reusing the same logic: force mc execute.
                // We duplicate a thin wrapper by setting a flag and calling the when step
                // pattern — but when_close_all is the only public entry. Use the module's
                // public pattern: `when I close all UDS clients` calls execute_multi_client_uds.
                // For Then steps that need events without that When, invoke it via a local
                // re-export pattern by calling the same code path through world state.
                //
                // Actually execute_multi_client_uds is private in uds_steps. Call the
                // public step function? It's not pub. So we must implement our own
                // multi-client drive for busy scenarios (drive_mc_live_busy) and for
                // simple cases use single-client execute_uds.
                //
                // For pure assertion Then steps after "I close all UDS clients",
                // mc_exit_code is already Some.
            }
        }
    } else if world.uds_exit_code.is_none() {
        uds_steps::execute_uds(world);
    }
}

fn agent_events(world: &QuectoWorld) -> Vec<String> {
    if world.mc_mode {
        // Flatten client 1 events as the primary stream; fall back to all.
        if let Some(ev) = world.mc_client_events.get(&1) {
            return ev.clone();
        }
        world
            .mc_client_events
            .values()
            .flat_map(|v| v.iter().cloned())
            .collect()
    } else {
        world.agent_events.clone()
    }
}

fn openai_text_body(content: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "chatcmpl-bdd-1060",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15
        }
    })
}

/// Mount sequential mock responses: N fast bodies, then one delayed body.
/// Used by busy-connect scenarios that need a completed first turn then a slow second.
fn mount_sequential_text_responses(world: &mut QuectoWorld, bodies: &[(&str, Duration)]) {
    assert!(
        world._wiremock_server_uri.is_some(),
        "mock server URI not set — ensure a config step ran first"
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();
        // Mount the fallback first, then one-shot responses in reverse order so
        // the first scenario response is the most recently registered match.
        for (i, (content, delay)) in bodies.iter().enumerate().rev() {
            let mut tmpl =
                wiremock::ResponseTemplate::new(200).set_body_json(openai_text_body(content));
            if !delay.is_zero() {
                tmpl = tmpl.set_delay(*delay);
            }
            let mut mock = wiremock::Mock::given(wiremock::matchers::method("POST"))
                .and(wiremock::matchers::path("/chat/completions"))
                .respond_with(tmpl);
            // Last response stays mounted forever; earlier ones are one-shot.
            if i + 1 < bodies.len() {
                mock = mock.up_to_n_times(1);
            }
            mock.mount(&server).await;
        }
        e2e_steps::rewrite_config_to_uri(world, &new_uri);
        // Keep server alive for the scenario.
        world.wiremock_server_ref = Some(Box::leak(Box::new(server)));
        world._wiremock_server_uri = Some(new_uri);
    });
    std::mem::forget(rt);
}

// ─── Given: oversized tool-call mock ──────────────────────────────────────────

#[given(
    "the mock LLM returns a tool call with arguments larger than the event line cap then a text response"
)]
fn given_mock_llm_oversized_tool_then_text(world: &mut QuectoWorld) {
    assert!(
        world._wiremock_server_uri.is_some(),
        "mock server URI not set"
    );
    let huge_args = format!(
        "{{\"command\":\"{}\"}}",
        "x".repeat(EVENT_LINE_CAP_BYTES + 4096)
    );
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let server = wiremock::MockServer::start().await;
        let new_uri = server.uri();

        let tool_call_body = serde_json::json!({
            "id": "chatcmpl-tool-huge",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_uds_bash_huge",
                        "type": "function",
                        "function": {
                            "name": "bash",
                            "arguments": huge_args
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });
        let text_body = openai_text_body("done after bulk tool");

        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(tool_call_body))
            .up_to_n_times(1)
            .with_priority(2)
            .mount(&server)
            .await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(text_body))
            .mount(&server)
            .await;

        e2e_steps::rewrite_config_to_uri(world, &new_uri);
        world.wiremock_server_ref = Some(Box::leak(Box::new(server)));
        world._wiremock_server_uri = Some(new_uri);
    });
    std::mem::forget(rt);
}

// ─── Given: completed turn with refs (single-client setup for get_message) ────

#[given("an idle persisted agent session containing an oversized prior assistant message")]
fn given_idle_session_with_oversized_message(world: &mut QuectoWorld) {
    seed_oversized_message(world);
}

#[given("a persisted agent session contains an oversized prior assistant message")]
fn given_session_with_oversized_message(world: &mut QuectoWorld) {
    seed_oversized_message(world);
}

#[given("a completed turn whose end-of-turn events identify messages by non-empty refs")]
fn given_completed_turn_with_refs(world: &mut QuectoWorld) {
    // Drive a full single-client UDS turn now so subsequent When steps can
    // issue get_message against the same live socket... but the standard
    // execute_uds closes the connection. Instead: queue prompt, execute, then
    // stash refs + full content from agent_end / get_messages-equivalent.
    //
    // For the on-demand lookup scenario the When step re-opens is not available
    // after close. Strategy: complete the turn via execute_uds, extract refs
    // from events, and also extract full content from the synthetic token /
    // known mock body so get_message assertions can be satisfied by re-driving
    // a second UDS session that still has history when persist/session is used.
    //
    // The feature uses no-session, so history dies with the process. The
    // scenario text is:
    //   Given ... completed turn ...
    //   When I request each end-of-turn message by its stable ref via get_message
    //   Then every get_message response should succeed...
    //
    // So the When must run get_message against a live agent that still holds
    // the ledger. We run a dedicated multi-client/persist session here:
    world.mc_mode = true;
    world.no_session = true;
    world._mc_persist = true;
    world.mc_connected_clients.push(1);
    let prompt = serde_json::json!({"type": "prompt", "message": "remember for get_message"});
    world
        .mc_client_commands
        .entry(1)
        .or_default()
        .push(prompt.to_string());

    // Ensure mock returns the body from prior given (text response already mounted).
    // Drive live: connect, prompt, wait agent_end, leave agent up for get_message.
    drive_mc_first_turn_keep_alive(world);

    let events = world.mc_client_events.get(&1).cloned().unwrap_or_default();
    let parsed = parse_events(&events);
    let agent_ends = events_of_type(&parsed, "agent_end");
    assert!(
        !agent_ends.is_empty(),
        "expected agent_end after completed turn; events: {events:#?}"
    );
    let refs = non_empty_refs(agent_ends[0]);
    assert!(
        !refs.is_empty(),
        "expected non-empty messageRefs on agent_end; got: {}",
        agent_ends[0]
    );
    world._bounded_message_refs = refs;
    // Also capture turn_end refs union.
    for te in events_of_type(&parsed, "turn_end") {
        for r in non_empty_refs(te) {
            if !world._bounded_message_refs.contains(&r) {
                world._bounded_message_refs.push(r);
            }
        }
    }
    world._mc_live_busy = true; // keep live handle semantics for get_message When
}

// ─── Given: multi-client agent with client 1 connected ────────────────────────

#[given("a multi-client UDS agent with client 1 connected")]
fn given_mc_agent_client1_connected(world: &mut QuectoWorld) {
    world.mc_mode = true;
    world.no_session = true;
    world._mc_persist = true;
    world.mc_connected_clients.push(1);
    world.mc_client_commands.entry(1).or_default();
    world.mc_client_events.entry(1).or_default();
    // Start agent and connect client 1 without closing.
    drive_mc_start_and_connect(world, &[1]);
    world._mc_live_busy = true;
}

// ─── When: multi-client busy / phased steps ───────────────────────────────────

#[when("I wait for the first turn to complete")]
fn when_wait_first_turn_complete(world: &mut QuectoWorld) {
    world._mc_live_busy = true;
    // Feature stacks "delay by N" then "returns text X". Stock steps leave a
    // single delayed mock; remount as (fast X, delayed X) so turn 1 completes
    // and turn 2 stays busy for concurrent connect/get_message.
    remount_busy_mock_if_needed(world);
    // Ensure agent is running with clients connected and first prompt sent.
    if world._mc_live_socket.is_none() {
        drive_mc_start_and_connect(world, &world.mc_connected_clients.clone());
        // Send any queued commands for connected clients.
        send_queued_commands_live(world);
    } else {
        send_queued_commands_live(world);
    }
    // Wait until client 1 sees agent_end.
    wait_client_agent_end(world, 1, Duration::from_secs(60));
}

#[when("I request the oversized message by its stable reference")]
fn when_request_oversized_message_by_ref(world: &mut QuectoWorld) {
    collect_paged_oversized_response(world);
}

#[given("the agent is processing a later turn")]
#[when("the agent is processing a later turn")]
fn when_agent_processing_later_turn(world: &mut QuectoWorld) {
    world._mc_live_busy = true;
    if world._mc_live_socket.is_none() {
        drive_mc_start_and_connect(world, &[1]);
    }
    if !world._mc_live_streams.contains_key(&1) {
        connect_client_live(world, 1);
    }
    let body = world
        ._bounded_expected_body
        .clone()
        .unwrap_or_else(|| "later turn".to_string());
    mount_sequential_text_responses(world, &[("later turn", Duration::from_secs(3))]);
    world._bounded_expected_body = Some(body);
    let cmd = serde_json::json!({"type":"prompt","message":"slow later turn"});
    let stream = world
        ._mc_live_streams
        .get_mut(&1)
        .expect("client 1 live stream for later turn");
    writeln!(stream, "{cmd}").expect("write later-turn prompt");
    stream.flush().expect("flush later-turn prompt");
    std::thread::sleep(Duration::from_millis(200));
}

#[when("another client requests the oversized message by its stable reference")]
fn when_another_client_requests_oversized_message(world: &mut QuectoWorld) {
    collect_paged_oversized_response(world);
}

#[when(expr = "client {int} connects while the agent is busy")]
fn when_client_connects_while_busy(world: &mut QuectoWorld, client_id: u32) {
    world._mc_live_busy = true;
    if world._mc_live_socket.is_none() {
        // Start agent, connect existing clients, send their commands so agent is busy.
        let already: Vec<u32> = world.mc_connected_clients.clone();
        drive_mc_start_and_connect(world, &already);
        send_queued_commands_live(world);
        // Brief pause so the second prompt is in-flight.
        std::thread::sleep(Duration::from_millis(200));
    } else {
        // Second prompt should already be queued on client 1 — send remaining cmds.
        send_queued_commands_live(world);
        std::thread::sleep(Duration::from_millis(200));
    }
    if !world.mc_connected_clients.contains(&client_id) {
        world.mc_connected_clients.push(client_id);
    }
    connect_client_live(world, client_id);
    // Busy-connect should push a get_messages snapshot immediately; drain a bit.
    drain_client_events(world, client_id, Duration::from_millis(800));
}

#[when("I record a non-empty message ref from the completed turn")]
fn when_record_message_ref(world: &mut QuectoWorld) {
    let events = world
        .mc_client_events
        .get(&1)
        .cloned()
        .unwrap_or_else(|| agent_events(world));
    let parsed = parse_events(&events);
    let mut refs = Vec::new();
    for ty in ["agent_end", "turn_end"] {
        for ev in events_of_type(&parsed, ty) {
            refs.extend(non_empty_refs(ev));
        }
    }
    refs.retain(|r| !r.is_empty());
    refs.sort();
    refs.dedup();
    assert!(
        !refs.is_empty(),
        "expected non-empty message refs from completed turn; events: {events:#?}"
    );
    // Prefer an assistant-looking ref if we can resolve via snapshot; else first.
    world._bounded_recorded_ref = Some(refs[0].clone());
    world._bounded_message_refs = refs;
}

#[when(expr = "client {int} requests get_message for the recorded ref")]
fn when_client_requests_get_message(world: &mut QuectoWorld, client_id: u32) {
    let mid = world
        ._bounded_recorded_ref
        .clone()
        .expect("no recorded message ref — call 'I record a non-empty message ref' first");
    let cmd = serde_json::json!({
        "type": "get_message",
        "id": "gm-busy-1",
        "messageId": mid
    });
    world
        .mc_client_commands
        .entry(client_id)
        .or_default()
        .push(cmd.to_string());
    // Send immediately on live stream.
    if let Some(stream) = world._mc_live_streams.get_mut(&client_id) {
        let line = format!("{cmd}\n");
        let _ = stream.write_all(line.as_bytes());
        let _ = stream.flush();
        // Mark command as sent so close doesn't re-send.
        if let Some(cmds) = world.mc_client_commands.get_mut(&client_id) {
            cmds.clear();
        }
    }
    drain_client_events(world, client_id, Duration::from_secs(3));
}

#[when("I request each end-of-turn message by its stable ref via get_message")]
fn when_request_each_ref_via_get_message(world: &mut QuectoWorld) {
    let refs = world._bounded_message_refs.clone();
    assert!(
        !refs.is_empty(),
        "no message refs recorded from completed turn"
    );
    // Use client 1 live stream (or connect a fetch client).
    let client_id = 1u32;
    if !world._mc_live_streams.contains_key(&client_id) {
        connect_client_live(world, client_id);
    }
    world._bounded_get_message_responses.clear();
    for (i, mid) in refs.iter().enumerate() {
        let id = format!("gm-ref-{i}");
        let cmd = serde_json::json!({
            "type": "get_message",
            "id": id,
            "messageId": mid
        });
        if let Some(stream) = world._mc_live_streams.get_mut(&client_id) {
            let line = format!("{cmd}\n");
            let _ = stream.write_all(line.as_bytes());
            let _ = stream.flush();
        }
        // Collect until we see the matching response.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            drain_client_events(world, client_id, Duration::from_millis(100));
            let events = world
                .mc_client_events
                .get(&client_id)
                .cloned()
                .unwrap_or_default();
            let found = events.iter().rev().find_map(|l| {
                let v: serde_json::Value = serde_json::from_str(l).ok()?;
                if v.get("type").and_then(|t| t.as_str()) == Some("response")
                    && v.get("command").and_then(|c| c.as_str()) == Some("get_message")
                    && v.get("id").and_then(|i| i.as_str()) == Some(id.as_str())
                {
                    Some(v)
                } else {
                    None
                }
            });
            if let Some(v) = found {
                world._bounded_get_message_responses.push(v);
                break;
            }
            if Instant::now() > deadline {
                panic!(
                    "timeout waiting for get_message response id={id} messageId={mid}; events: {events:#?}"
                );
            }
        }
    }
}

#[when("a sub-agent completes a turn visible on the parent event stream")]
fn when_subagent_completes_turn_on_parent(world: &mut QuectoWorld) {
    // Exercise the real child socket -> monitor -> parent broadcast path.
    let dir = tempfile::tempdir().expect("child socket tempdir");
    let socket = dir.path().join("worker.sock");
    let rt = tokio::runtime::Runtime::new().expect("monitor runtime");
    let forwarded = rt.block_on(async {
        let listener = tokio::net::UnixListener::bind(&socket).expect("bind child socket");
        let registry = quecto::infrastructure::tools::subagent_registry::new_registry();
        registry.lock().expect("registry").insert(
            "worker".into(),
            quecto::infrastructure::tools::subagent_registry::SubagentEntry::new(socket.clone(), 0),
        );
        let (tx, mut rx) = tokio::sync::broadcast::channel(8);
        let monitor = quecto::infrastructure::tools::subagent_monitor::spawn_monitor_task(
            "worker".into(),
            socket,
            registry,
            None,
            Some(tx),
            None,
        );
        let (child, _) = listener.accept().await.expect("monitor connected");
        let (read_half, mut write_half) = child.into_split();
        let mut child_reader = tokio::io::BufReader::new(read_half);
        let _ = quecto_line_io::read_frame(&mut child_reader, PROTOCOL_FRAME_CAP_BYTES)
            .await
            .expect("monitor hello");
        // Drive the child-side production emitter rather than fabricating the
        // event shape: TurnCompleted messages are converted to refs-only
        // subagent_messages_appended on the child's own stream, then the monitor
        // re-stamps that real child event onto the parent broadcast path.
        let body = world
            ._bounded_expected_body
            .clone()
            .unwrap_or_else(|| "child turn body".to_string());
        let child_messages = vec![quecto::domain::message::Message::assistant(&body, vec![])];
        let mut child_bytes = Vec::new();
        quecto::interface::cli::uds_cancel::forward_progress_event(
            quecto::domain::agent::AgentProgressEvent::TurnCompleted {
                messages: child_messages.into(),
            },
            &mut child_bytes,
        )
        .await;
        let child_line = String::from_utf8(child_bytes)
            .expect("child event utf8")
            .lines()
            .find(|line| line.contains("\"type\":\"subagent_messages_appended\""))
            .expect("production child subagent_messages_appended")
            .to_string();
        let child_json: serde_json::Value = serde_json::from_str(&child_line).unwrap();
        let produced_refs = non_empty_refs(&child_json);
        assert!(
            !produced_refs.is_empty(),
            "production child event must carry refs: {child_line}"
        );
        assert!(
            !content_re_carried(&child_json),
            "child event must be refs-only: {child_line}"
        );
        quecto_line_io::write_frame(
            &mut write_half,
            child_line.as_bytes(),
            PROTOCOL_FRAME_CAP_BYTES,
        )
        .await
        .expect("write child event");
        let line = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let line = rx.recv().await.expect("parent broadcast");
                if serde_json::from_str::<serde_json::Value>(&line)
                    .ok()
                    .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_owned))
                    .as_deref()
                    == Some("subagent_messages_appended")
                {
                    break line;
                }
            }
        })
        .await
        .expect("forwarded child event");
        monitor.abort();
        line
    });
    world.mc_client_events.entry(1).or_default().push(forwarded);
    world._bounded_subagent_appended = true;
}

// ─── Then: size bounds ────────────────────────────────────────────────────────

#[then("the turn_end event should stay well under the frame size limit")]
fn then_turn_end_well_under_frame(world: &mut QuectoWorld) {
    ensure_uds_events(world);
    let lines = agent_events(world);
    let len = event_line_len(&lines, "turn_end")
        .unwrap_or_else(|| panic!("no turn_end event in: {lines:#?}"));
    assert!(
        len < WELL_UNDER_FRAME,
        "turn_end must stay well under the frame size limit ({WELL_UNDER_FRAME}); got {len} bytes (frame cap {PROTOCOL_FRAME_CAP_BYTES})"
    );
    assert!(
        len < PROTOCOL_FRAME_CAP_BYTES,
        "turn_end must be under PROTOCOL_FRAME_CAP_BYTES; got {len}"
    );
}

#[then("the agent_end event should stay well under the frame size limit")]
fn then_agent_end_well_under_frame(world: &mut QuectoWorld) {
    ensure_uds_events(world);
    let lines = agent_events(world);
    let len = event_line_len(&lines, "agent_end")
        .unwrap_or_else(|| panic!("no agent_end event in: {lines:#?}"));
    assert!(
        len < WELL_UNDER_FRAME,
        "agent_end must stay well under the frame size limit ({WELL_UNDER_FRAME}); got {len} bytes"
    );
}

#[then("the turn_end event should stay under the hard event line cap")]
fn then_turn_end_under_line_cap(world: &mut QuectoWorld) {
    ensure_uds_events(world);
    let lines = agent_events(world);
    let len = event_line_len(&lines, "turn_end")
        .unwrap_or_else(|| panic!("no turn_end event in: {lines:#?}"));
    // Cap includes trailing newline on the wire; our stored lines may omit it.
    assert!(
        len < EVENT_LINE_CAP_BYTES,
        "turn_end must stay under EVENT_LINE_CAP_BYTES ({EVENT_LINE_CAP_BYTES}); got {len}"
    );
}

#[then("the agent_end event should stay under the hard event line cap")]
fn then_agent_end_under_line_cap(world: &mut QuectoWorld) {
    ensure_uds_events(world);
    let lines = agent_events(world);
    let len = event_line_len(&lines, "agent_end")
        .unwrap_or_else(|| panic!("no agent_end event in: {lines:#?}"));
    assert!(
        len < EVENT_LINE_CAP_BYTES,
        "agent_end must stay under EVENT_LINE_CAP_BYTES ({EVENT_LINE_CAP_BYTES}); got {len}"
    );
}

// ─── Then: no full content re-carry ───────────────────────────────────────────

#[then("the turn_end event should not re-carry the full assistant content")]
fn then_turn_end_no_full_content(world: &mut QuectoWorld) {
    ensure_uds_events(world);
    let lines = agent_events(world);
    let parsed = parse_events(&lines);
    let turn_ends = events_of_type(&parsed, "turn_end");
    assert!(
        !turn_ends.is_empty(),
        "expected turn_end; events: {lines:#?}"
    );
    for te in &turn_ends {
        assert!(
            !content_re_carried(te),
            "turn_end must not re-carry full assistant content: {te}"
        );
        let c = te
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("");
        assert!(
            c.is_empty(),
            "turn_end message.content must be empty after #1060; got {} chars",
            c.len()
        );
    }
}

#[then("the agent_end event should not re-carry full message content")]
fn then_agent_end_no_full_content(world: &mut QuectoWorld) {
    ensure_uds_events(world);
    let lines = agent_events(world);
    let parsed = parse_events(&lines);
    let agent_ends = events_of_type(&parsed, "agent_end");
    assert!(
        !agent_ends.is_empty(),
        "expected agent_end; events: {lines:#?}"
    );
    for ae in &agent_ends {
        assert!(
            !content_re_carried(ae),
            "agent_end must not re-carry full message content: {ae}"
        );
        if let Some(msgs) = ae.get("messages").and_then(|m| m.as_array()) {
            assert!(
                msgs.is_empty()
                    || msgs.iter().all(|m| {
                        m.get("content")
                            .and_then(|c| c.as_str())
                            .unwrap_or("")
                            .is_empty()
                    }),
                "agent_end.messages must be empty or emptied content after #1060: {ae}"
            );
        }
    }
}

// ─── Then: non-empty message refs ─────────────────────────────────────────────

#[then("the turn_end event should identify the turn messages by non-empty message refs")]
fn then_turn_end_nonempty_refs(world: &mut QuectoWorld) {
    ensure_uds_events(world);
    let lines = agent_events(world);
    let parsed = parse_events(&lines);
    let turn_ends = events_of_type(&parsed, "turn_end");
    assert!(
        !turn_ends.is_empty(),
        "expected turn_end; events: {lines:#?}"
    );
    let refs = non_empty_refs(turn_ends[0]);
    assert!(
        !refs.is_empty(),
        "turn_end must identify messages by non-empty messageRefs; got: {}",
        turn_ends[0]
    );
    world._bounded_message_refs = refs;
}

#[then("the agent_end event should identify the run messages by non-empty message refs")]
fn then_agent_end_nonempty_refs(world: &mut QuectoWorld) {
    ensure_uds_events(world);
    let lines = agent_events(world);
    let parsed = parse_events(&lines);
    let agent_ends = events_of_type(&parsed, "agent_end");
    assert!(
        !agent_ends.is_empty(),
        "expected agent_end; events: {lines:#?}"
    );
    let refs = non_empty_refs(agent_ends[0]);
    assert!(
        !refs.is_empty(),
        "agent_end must identify messages by non-empty messageRefs; got: {}",
        agent_ends[0]
    );
    // Merge into stored refs for later match steps.
    for r in refs {
        if !world._bounded_message_refs.contains(&r) {
            world._bounded_message_refs.push(r);
        }
    }
}

#[then("the agent_end message refs should cover assistant tool-call and tool-result roles")]
fn then_agent_end_refs_cover_tool_roles(world: &mut QuectoWorld) {
    ensure_uds_events(world);
    let lines = agent_events(world);
    let parsed = parse_events(&lines);
    let agent_ends = events_of_type(&parsed, "agent_end");
    assert!(
        !agent_ends.is_empty(),
        "expected agent_end; events: {lines:#?}"
    );
    let refs = non_empty_refs(agent_ends[0]);
    assert!(
        refs.len() >= 2,
        "expected refs covering tool-call assistant + tool result (at least 2); got {refs:?} from {}",
        agent_ends[0]
    );

    // Prefer resolving roles via get_message / get_messages if present;
    // otherwise infer from tool_execution events + ref count.
    let mut roles: Vec<String> = Vec::new();
    if let Some(gm) = uds_steps::find_agent_response(world, "get_messages") {
        if let Some(msgs) = gm
            .get("data")
            .and_then(|d| d.get("messages"))
            .and_then(|m| m.as_array())
        {
            for m in msgs {
                if let Some(id) = m.get("id").and_then(|i| i.as_str()) {
                    if refs.iter().any(|r| r == id) {
                        if let Some(role) = m.get("role").and_then(|r| r.as_str()) {
                            roles.push(role.to_string());
                        }
                    }
                }
            }
        }
    }

    // The live stream's per-message events are another authoritative resolution
    // source: match their stable ids to the end-of-turn refs.
    if roles.is_empty() {
        for event in &parsed {
            let messages = event
                .get("messages")
                .and_then(|m| m.as_array())
                .cloned()
                .unwrap_or_else(|| event.get("message").cloned().into_iter().collect());
            for message in messages {
                let id = message.get("id").and_then(|v| v.as_str()).unwrap_or("");
                if refs.iter().any(|r| r == id) {
                    if let Some(role) = message.get("role").and_then(|v| v.as_str()) {
                        roles.push(role.to_owned());
                    }
                }
            }
        }
    }
    assert!(
        !roles.is_empty(),
        "could not resolve tool refs {refs:?} to streamed messages"
    );
    // Also inspect tool events for evidence of a tool-using turn.
    // Oversized args may fail before start is emitted (only tool_execution_end),
    // but agent_end still carries refs for assistant tool-call + tool-result.
    let has_tool_event = parsed.iter().any(|e| {
        matches!(
            e.get("type").and_then(|t| t.as_str()),
            Some("tool_execution_start" | "tool_execution_end")
        )
    });
    assert!(
        has_tool_event,
        "expected tool_execution_start/end in a tool-using turn; events: {lines:#?}"
    );

    let has_assistant = roles.iter().any(|r| r == "assistant");
    let has_tool = roles.iter().any(|r| r == "tool");
    assert!(
        has_assistant && has_tool,
        "agent_end message refs must cover assistant tool-call and tool-result roles; roles={roles:?} refs={refs:?}"
    );
    // (refs.len() >= 2 already asserted above; has_assistant && has_tool implies it.)
}

// ─── Then: get_messages id parity ─────────────────────────────────────────────

#[then(
    "the get_messages response messages should each carry a non-empty stable message identifier"
)]
fn then_get_messages_stable_ids(world: &mut QuectoWorld) {
    ensure_uds_events(world);
    let resp =
        uds_steps::find_agent_response(world, "get_messages").expect("no get_messages response");
    let msgs = resp
        .get("data")
        .and_then(|d| d.get("messages"))
        .and_then(|m| m.as_array())
        .expect("get_messages.data.messages array");
    assert!(
        !msgs.is_empty(),
        "get_messages returned no messages: {resp}"
    );
    for m in msgs {
        let id = m.get("id").and_then(|i| i.as_str()).unwrap_or("");
        assert!(
            !id.is_empty(),
            "each get_messages message must carry a non-empty stable id; message={m}"
        );
    }
    // Stash ids for match step.
    world._bounded_get_messages_ids = msgs
        .iter()
        .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(str::to_string))
        .collect();
}

#[then("the get_messages message identifiers should match the end-of-turn message refs")]
fn then_get_messages_ids_match_refs(world: &mut QuectoWorld) {
    ensure_uds_events(world);
    let lines = agent_events(world);
    let parsed = parse_events(&lines);
    let mut eot_refs = Vec::new();
    for ty in ["agent_end", "turn_end"] {
        for ev in events_of_type(&parsed, ty) {
            eot_refs.extend(non_empty_refs(ev));
        }
    }
    eot_refs.sort();
    eot_refs.dedup();
    assert!(!eot_refs.is_empty(), "no end-of-turn refs to match");

    let gm_ids = if world._bounded_get_messages_ids.is_empty() {
        let resp = uds_steps::find_agent_response(world, "get_messages")
            .expect("no get_messages response");
        resp.get("data")
            .and_then(|d| d.get("messages"))
            .and_then(|m| m.as_array())
            .map(|msgs| {
                msgs.iter()
                    .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        world._bounded_get_messages_ids.clone()
    };

    for r in &eot_refs {
        assert!(
            gm_ids.iter().any(|id| id == r),
            "end-of-turn ref {r} missing from get_messages ids {gm_ids:?}"
        );
    }
}

// ─── Then: busy-connect snapshot ──────────────────────────────────────────────

#[then(
    expr = "client {int} should have received a get_messages snapshot with non-empty stable message identifiers"
)]
fn then_client_snapshot_stable_ids(world: &mut QuectoWorld, client_id: u32) {
    if world.mc_exit_code.is_none() && world._mc_live_busy {
        // Finalize: drain remaining, close streams.
        finalize_mc_live(world);
    }
    let events = world
        .mc_client_events
        .get(&client_id)
        .cloned()
        .unwrap_or_default();
    let snapshot = events.iter().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        if v.get("type").and_then(|t| t.as_str()) == Some("response")
            && v.get("command").and_then(|c| c.as_str()) == Some("get_messages")
        {
            Some(v)
        } else {
            None
        }
    });
    let snap = snapshot.unwrap_or_else(|| {
        panic!("client {client_id} expected get_messages snapshot; events: {events:#?}")
    });
    // data may be {messages:[...], snapshot:true} or legacy array
    let msgs = snap
        .get("data")
        .and_then(|d| {
            d.get("messages")
                .and_then(|m| m.as_array())
                .cloned()
                .or_else(|| d.as_array().cloned())
        })
        .unwrap_or_default();
    assert!(
        !msgs.is_empty(),
        "busy-connect get_messages snapshot should include messages; got {snap}"
    );
    let mut ids = Vec::new();
    for m in &msgs {
        let id = m.get("id").and_then(|i| i.as_str()).unwrap_or("");
        assert!(!id.is_empty(), "snapshot message missing stable id: {m}");
        ids.push(id.to_string());
    }
    world._bounded_snapshot_ids = ids;
}

#[then(
    "those snapshot message identifiers should match the completed turn's end-of-turn message refs"
)]
fn then_snapshot_ids_match_eot_refs(world: &mut QuectoWorld) {
    let lines = world.mc_client_events.get(&1).cloned().unwrap_or_default();
    let parsed = parse_events(&lines);
    // First completed turn's agent_end (before the slow second).
    let agent_ends = events_of_type(&parsed, "agent_end");
    assert!(
        !agent_ends.is_empty(),
        "client 1 should have agent_end from completed turn; events: {lines:#?}"
    );
    let eot_refs = non_empty_refs(agent_ends[0]);
    assert!(!eot_refs.is_empty(), "completed turn agent_end has no refs");
    let snap_ids = &world._bounded_snapshot_ids;
    for r in &eot_refs {
        assert!(
            snap_ids.iter().any(|id| id == r),
            "end-of-turn ref {r} missing from busy-connect snapshot ids {snap_ids:?}"
        );
    }
}

// ─── Then: get_message responses ──────────────────────────────────────────────

#[then("every oversized-message response fragment should stay within the protocol frame cap")]
#[then(
    "every oversized-message response fragment received by that client should stay within the protocol frame cap"
)]
fn then_oversized_fragments_bounded(world: &mut QuectoWorld) {
    assert!(
        !world._bounded_get_message_responses.is_empty(),
        "no oversized get_message response fragments recorded"
    );
    for response in &world._bounded_get_message_responses {
        let line = serde_json::to_string(response).expect("response serializes");
        assert!(
            line.len() <= EVENT_LINE_CAP_BYTES,
            "response fragment must stay within protocol frame cap: {} > {}: {response}",
            line.len(),
            EVENT_LINE_CAP_BYTES
        );
    }
}

#[then("the response fragments should reassemble the full message content")]
#[then("that client should reassemble the full oversized message content")]
fn then_oversized_fragments_reassemble(world: &mut QuectoWorld) {
    let expected = world
        ._bounded_expected_body
        .as_ref()
        .expect("expected oversized body");
    assert_eq!(reassembled_oversized_content(world), *expected);
}

#[then("the UDS client connection should remain open")]
#[then("that client should remain connected while the agent is busy")]
fn then_oversized_client_remains_connected(world: &mut QuectoWorld) {
    let client_id = world
        ._bounded_oversized_client_id
        .expect("oversized response client id");
    let stream = world
        ._mc_live_streams
        .get_mut(&client_id)
        .expect("live UDS client stream for connection probe");
    writeln!(
        stream,
        "{}",
        serde_json::json!({"type":"get_state","id":"oversized-connection-probe"})
    )
    .expect("post-recovery get_state write must succeed");
    stream
        .flush()
        .expect("post-recovery probe flush must succeed");
    let mut reader = BufReader::new(stream.try_clone().expect("clone UDS stream"));
    reader
        .get_mut()
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set read timeout");
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .expect("post-recovery get_state response should arrive on open connection");
        assert!(
            n > 0,
            "UDS connection closed before post-recovery get_state response"
        );
        if line.contains("oversized-connection-probe") {
            break;
        }
    }
}

#[then("every get_message response should succeed with the full message content for its ref")]
fn then_every_get_message_succeeds_full(world: &mut QuectoWorld) {
    let responses = &world._bounded_get_message_responses;
    assert!(!responses.is_empty(), "no get_message responses recorded");
    for resp in responses {
        assert_eq!(
            resp.get("success").and_then(|s| s.as_bool()),
            Some(true),
            "get_message should succeed: {resp}"
        );
        let data = resp.get("data").expect("get_message data");
        // Full content: either non-empty content string, or toolCalls present.
        let content = data.get("content").and_then(|c| c.as_str()).unwrap_or("");
        let tool_calls = data
            .get("toolCalls")
            .and_then(|t| t.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false);
        assert!(
            !content.is_empty() || tool_calls || data.get("toolCallId").is_some(),
            "get_message must return full message content for its ref: {resp}"
        );
    }
    // Beyond per-ref resolvability, at least one resolved message must carry the
    // exact mock body — proving real content round-trips, not just non-emptiness.
    if let Some(expected) = &world._bounded_expected_body {
        assert!(
            responses.iter().any(|resp| resp
                .get("data")
                .and_then(|d| d.get("content"))
                .and_then(|c| c.as_str())
                .is_some_and(|content| content.contains(expected.as_str()))),
            "at least one get_message must return the expected body {expected:?}"
        );
    }
}

#[then("every get_message response should round-trip the requested message identifier")]
fn then_every_get_message_roundtrips_id(world: &mut QuectoWorld) {
    let refs = &world._bounded_message_refs;
    let responses = &world._bounded_get_message_responses;
    assert_eq!(
        responses.len(),
        refs.len(),
        "expected one get_message response per ref"
    );
    for (resp, expected_id) in responses.iter().zip(refs.iter()) {
        let got = resp
            .get("data")
            .and_then(|d| d.get("id"))
            .and_then(|i| i.as_str())
            .unwrap_or("");
        assert_eq!(
            got, expected_id,
            "get_message must round-trip message id; expected {expected_id}, got {got} in {resp}"
        );
    }
}

#[then(
    expr = "client {int} should have received a successful get_message response for the requested ref"
)]
fn then_client_get_message_success(world: &mut QuectoWorld, client_id: u32) {
    if world.mc_exit_code.is_none() && world._mc_live_busy {
        finalize_mc_live(world);
    }
    let events = world
        .mc_client_events
        .get(&client_id)
        .cloned()
        .unwrap_or_default();
    let mid = world._bounded_recorded_ref.as_deref().unwrap_or("");
    let resp = events.iter().find_map(|l| {
        let v: serde_json::Value = serde_json::from_str(l).ok()?;
        if v.get("type").and_then(|t| t.as_str()) == Some("response")
            && v.get("command").and_then(|c| c.as_str()) == Some("get_message")
            && v.get("success").and_then(|s| s.as_bool()) == Some(true)
        {
            Some(v)
        } else {
            None
        }
    });
    let resp = resp.unwrap_or_else(|| {
        panic!(
            "client {client_id} expected successful get_message for ref {mid}; events: {events:#?}"
        )
    });
    world._bounded_get_message_responses = vec![resp];
}

#[then("the get_message response should carry full content for the requested ref")]
fn then_get_message_full_content(world: &mut QuectoWorld) {
    let resp = world
        ._bounded_get_message_responses
        .first()
        .expect("no get_message response");
    let data = resp.get("data").expect("data");
    let content = data.get("content").and_then(|c| c.as_str()).unwrap_or("");
    assert!(
        !content.is_empty(),
        "get_message should carry full content; got {resp}"
    );
    // Prefer matching known mock bodies when available.
    if let Some(expected) = &world._bounded_expected_body {
        assert!(
            content.contains(expected.as_str()) || content == expected.as_str(),
            "get_message content should include expected body {expected:?}; got {content:?}"
        );
    }
    let mid = world._bounded_recorded_ref.as_deref().unwrap_or("");
    if !mid.is_empty() {
        let got = data.get("id").and_then(|i| i.as_str()).unwrap_or("");
        assert_eq!(got, mid, "get_message id must match requested ref");
    }
}

// ─── Then: footer metadata ────────────────────────────────────────────────────

#[then("the turn_end event should include numeric usage totals when usage is present")]
fn then_turn_end_usage_totals(world: &mut QuectoWorld) {
    ensure_uds_events(world);
    let lines = agent_events(world);
    let parsed = parse_events(&lines);
    let turn_ends = events_of_type(&parsed, "turn_end");
    assert!(
        !turn_ends.is_empty(),
        "expected turn_end; events: {lines:#?}"
    );
    // Mock OpenAI responses include usage, so the producer should surface it.
    let te = turn_ends[0];
    let usage = te
        .get("message")
        .and_then(|m| m.get("usage"))
        .expect("configured OpenAI mock supplies usage; turn_end must preserve it");
    assert!(
        usage.get("input").and_then(|v| v.as_u64()).is_some()
            || usage.get("total").and_then(|v| v.as_u64()).is_some(),
        "usage must contain numeric totals: {te}"
    );
}

// ─── Then: subagent path ──────────────────────────────────────────────────────

#[then(
    "the parent stream's subagent_messages_appended event should identify messages by non-empty message refs"
)]
fn then_parent_subagent_nonempty_refs(world: &mut QuectoWorld) {
    if world.mc_exit_code.is_none() && world._mc_live_busy {
        finalize_mc_live(world);
    }
    let lines = world
        .mc_client_events
        .get(&1)
        .cloned()
        .unwrap_or_else(|| agent_events(world));
    let parsed = parse_events(&lines);
    let events = events_of_type(&parsed, "subagent_messages_appended");
    assert!(
        !events.is_empty(),
        "expected subagent_messages_appended on parent stream; events: {lines:#?}"
    );
    let refs = non_empty_refs(events[0]);
    assert!(
        !refs.is_empty(),
        "subagent_messages_appended must have non-empty messageRefs: {}",
        events[0]
    );
}

#[then(
    "the parent stream's subagent_messages_appended event should not re-carry full message content"
)]
fn then_parent_subagent_no_full_content(world: &mut QuectoWorld) {
    let lines = world
        .mc_client_events
        .get(&1)
        .cloned()
        .unwrap_or_else(|| agent_events(world));
    let parsed = parse_events(&lines);
    let events = events_of_type(&parsed, "subagent_messages_appended");
    assert!(!events.is_empty(), "expected subagent_messages_appended");
    for ev in events {
        assert!(
            !content_re_carried(ev),
            "subagent_messages_appended must not re-carry full content: {ev}"
        );
    }
}

#[then(
    "the parent stream's subagent_messages_appended event should stay well under the frame size limit"
)]
fn then_parent_subagent_well_under_frame(world: &mut QuectoWorld) {
    let lines = world
        .mc_client_events
        .get(&1)
        .cloned()
        .unwrap_or_else(|| agent_events(world));
    let len = event_line_len(&lines, "subagent_messages_appended")
        .expect("no subagent_messages_appended line");
    assert!(
        len < WELL_UNDER_FRAME,
        "subagent_messages_appended must stay well under frame ({WELL_UNDER_FRAME}); got {len}"
    );
}

// ─── Live multi-client driver (busy-connect / phased prompts) ─────────────────

fn drive_mc_start_and_connect(world: &mut QuectoWorld, clients: &[u32]) {
    if world._mc_live_socket.is_some() {
        return;
    }
    let base = world
        .cli_context
        .base_dir
        .clone()
        .expect("no base dir — add 'Given a temp base directory'");
    if !base.join("config.json").exists() {
        world.agent_stderr = "config not found".to_string();
        world.mc_exit_code = Some(1);
        return;
    }

    // Reuse build + spawn from uds_steps via the public multi-client path pieces.
    // build_uds_agent / mc_spawn_agent are private — duplicate the essential spawn
    // by calling execute_multi_client only after live setup is impossible.
    // Instead: use a local spawn that mirrors mc_spawn_agent using public APIs.
    spawn_mc_agent_live(world, &base);

    for &cid in clients {
        if !world.mc_connected_clients.contains(&cid) {
            world.mc_connected_clients.push(cid);
        }
        connect_client_live(world, cid);
    }
}

fn spawn_mc_agent_live(world: &mut QuectoWorld, base: &std::path::Path) {
    use quecto::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
    use quecto::domain::session::Session;
    use quecto::infrastructure::config::Config;
    use quecto::infrastructure::security::sandbox::Sandbox;
    use quecto::infrastructure::tools::registry::ToolRegistryImpl;
    use quecto::interface::cli::build_agent_provider;
    use quecto::interface::cli::provider_reload::{ProviderReloadInputs, seeded_provider_reload};
    use quecto::interface::cli::uds::{UdsLoopArgs, run_uds_loop};

    let env_overrides: HashMap<String, String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("QUECTO_"))
        .collect();
    let config_path = base.join("config.json");
    let config = match Config::load_with_env(config_path.to_str().unwrap_or(""), &env_overrides) {
        Ok(c) => c,
        Err(e) => {
            world.agent_stderr = format!("failed to load config: {e}");
            world.mc_exit_code = Some(1);
            return;
        }
    };
    let http_client = reqwest::Client::new();
    let provider = match build_agent_provider(&config, base, &http_client) {
        Ok(p) => p,
        Err(e) => {
            world.agent_stderr = format!("provider error: {e}");
            world.mc_exit_code = Some(1);
            return;
        }
    };
    let mut provider_reload = seeded_provider_reload(&config_path, provider.clone());
    let provider_reload_inputs =
        ProviderReloadInputs::new(config_path, base.to_path_buf(), env_overrides, http_client);
    let workspace = std::path::PathBuf::from(config.workspace_path());
    let model = config.agents.defaults.model.clone();
    let sandbox = Sandbox::new(
        Some(workspace.clone()),
        config.agents.defaults.restrict_to_workspace,
    );
    let exec_settings = ToolRegistryImpl::exec_registry_settings_from_config(&config);
    let mut registry = quecto::infrastructure::extensions::native::build_official_tool_registry(
        workspace,
        sandbox,
        quecto::infrastructure::tools::bash::ExecOptions {
            max_capture_bytes: exec_settings,
            ..Default::default()
        },
    );
    let ext_registry = quecto::infrastructure::extensions::registry::ExtensionRegistry::new();
    quecto::interface::shared::register_bundled_native_extension_tools(
        &mut registry,
        &ext_registry,
    );
    let ephemeral = world.no_session || world.session_name.as_deref() == Some("-");
    let session_key = if ephemeral {
        String::new()
    } else {
        Session::build_key("cli", world.session_name.as_deref().unwrap_or("default"))
    };
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: model.clone(),
        max_tokens: config.agents.defaults.max_tokens,
        temperature: config.agents.defaults.temperature,
        spill_store: None,
        session_key: session_key.clone(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: config.agents.defaults.max_context_tokens,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: quecto::domain::tool::ToolProfileContext::Parent,
    });
    if world._uds_streaming_enabled {
        agent.set_streaming(true);
    }
    let ext_reg = std::sync::Arc::new(std::sync::Mutex::new(ext_registry));
    let socket_path = base.join("mc-bounded-test-agent.sock");
    let _ = std::fs::remove_file(&socket_path);
    let sp = socket_path.clone();
    let base_for_thread = base.to_path_buf();
    let persist = world._mc_persist;
    let handle = std::thread::spawn(move || {
        run_uds_loop(UdsLoopArgs {
            agent,
            base_dir: &base_for_thread,
            workspace: &base_for_thread,
            session_key,
            model,
            ephemeral,
            system_prompt: String::new(),
            socket_path: sp,
            socket_override: None,
            session_store_override: None,
            ext_registry: Some(ext_reg),
            persist,
            notification_rx: None,
            subagent_registry: None,
            container_registry: None,
            workflow_state: None,
            workflow_config: None,
            broadcast_tx: None,
            provider_reload: Some(&mut provider_reload),
            provider_reload_inputs: Some(&provider_reload_inputs),
        })
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !socket_path.exists() {
        if Instant::now() > deadline {
            world.agent_stderr = "timeout waiting for socket".to_string();
            world.mc_exit_code = Some(1);
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(50));
    world._mc_live_socket = Some(socket_path);
    world._mc_live_handle = Some(handle);
}

fn connect_client_live(world: &mut QuectoWorld, client_id: u32) {
    if world._mc_live_streams.contains_key(&client_id) {
        return;
    }
    let Some(socket_path) = world._mc_live_socket.clone() else {
        world.agent_stderr = "no live socket".to_string();
        return;
    };
    match UnixStream::connect(&socket_path) {
        Ok(s) => {
            s.set_read_timeout(Some(Duration::from_millis(200))).ok();
            s.set_nonblocking(false).ok();
            world._mc_live_streams.insert(client_id, s);
            world.mc_client_events.entry(client_id).or_default();
        }
        Err(e) => {
            world.agent_stderr = format!("client {client_id} connect failed: {e}");
            world.mc_exit_code = Some(1);
        }
    }
}

fn send_queued_commands_live(world: &mut QuectoWorld) {
    let commands: HashMap<u32, Vec<String>> = world.mc_client_commands.clone();
    for (cid, cmds) in commands {
        if cmds.is_empty() {
            continue;
        }
        if let Some(stream) = world._mc_live_streams.get_mut(&cid) {
            for cmd in &cmds {
                let _ = stream.write_all(format!("{cmd}\n").as_bytes());
            }
            let _ = stream.flush();
            // Clear sent commands so we don't re-send.
            if let Some(q) = world.mc_client_commands.get_mut(&cid) {
                q.clear();
            }
        }
    }
}

fn drain_client_events(world: &mut QuectoWorld, client_id: u32, budget: Duration) {
    let Some(stream) = world._mc_live_streams.get(&client_id) else {
        return;
    };
    let Ok(reader_stream) = stream.try_clone() else {
        return;
    };
    reader_stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .ok();
    let mut reader = BufReader::new(reader_stream);
    let deadline = Instant::now() + budget;
    let events = world.mc_client_events.entry(client_id).or_default();
    while Instant::now() < deadline {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let line = line.trim_end().to_string();
                if !line.is_empty() {
                    events.push(line);
                }
            }
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                // keep draining until budget
            }
            Err(_) => break,
        }
    }
}

fn wait_client_agent_end(world: &mut QuectoWorld, client_id: u32, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    // Scan only lines that arrived since the last tick. Cloning the whole event
    // buffer each poll re-copies (and re-scans) every previously seen line; in
    // the oversized-message scenarios those lines are megabytes each, which
    // turns this wait into quadratic work and can outrun the timeout on a
    // loaded runner even though the agent is behaving correctly.
    let mut scanned = 0usize;
    // Once we have seen a refs-only turn_end for this run, keep draining a short
    // grace window for agent_end — but do not fail the seed if the terminal
    // agent_end is delayed under CI load after an 8 MiB mock body. Downstream
    // steps only need the stable messageRefs (available on turn_end after #1060).
    let mut saw_turn_end_with_refs_at: Option<Instant> = None;
    // `workflow_idle` is emitted *after* prompt success handling (response ok +
    // idle drain). Under writer-task / CI scheduling it can land in the client
    // buffer while `turn_end`/`agent_end` (the only carriers of messageRefs for
    // `completed_turn_refs`) are still in flight or still unread. Returning on
    // bare `workflow_idle` caused the issue-1094 seed step to flake on PR #1197
    // with drained events ending at response + workflow_idle and no refs.
    // Treat workflow_idle as completion only once refs are already observed.
    let mut saw_workflow_idle = false;
    loop {
        drain_client_events(world, client_id, Duration::from_millis(200));
        if let Some(events) = world.mc_client_events.get(&client_id) {
            for line in events.iter().skip(scanned) {
                if line.contains(r#""type":"agent_end""#) {
                    return;
                }
                if line.contains(r#""type":"workflow_idle""#) {
                    saw_workflow_idle = true;
                }
                if saw_turn_end_with_refs_at.is_none()
                    && line.contains(r#""type":"turn_end""#)
                    && (line.contains("messageRefs") || line.contains("message_refs"))
                {
                    saw_turn_end_with_refs_at = Some(Instant::now());
                }
            }
            scanned = events.len();
        }
        // Refs already on turn_end: return immediately if the post-turn idle
        // boundary has also arrived, otherwise keep a short grace for agent_end.
        if let Some(seen_at) = saw_turn_end_with_refs_at {
            if saw_workflow_idle || seen_at.elapsed() >= Duration::from_secs(2) {
                return;
            }
        }
        if Instant::now() > deadline {
            let events = world
                .mc_client_events
                .get(&client_id)
                .cloned()
                .unwrap_or_default();
            panic!(
                "timeout waiting for agent_end on client {client_id}; events: {events:#?}; stderr={}",
                world.agent_stderr
            );
        }
    }
}

fn drive_mc_first_turn_keep_alive(world: &mut QuectoWorld) {
    drive_mc_start_and_connect(world, &world.mc_connected_clients.clone());
    send_queued_commands_live(world);
    wait_client_agent_end(world, 1, Duration::from_secs(60));
}

fn drive_mc_live_busy(world: &mut QuectoWorld) {
    // Used when Then steps need finalize without prior close.
    finalize_mc_live(world);
}

pub(crate) fn finalize_mc_live_pub(world: &mut QuectoWorld) {
    finalize_mc_live(world);
}

fn finalize_mc_live(world: &mut QuectoWorld) {
    // Drain remaining events, shut down streams, tear down agent.
    let clients: Vec<u32> = world._mc_live_streams.keys().copied().collect();
    for cid in clients {
        // Send any leftover commands first.
        send_queued_commands_live(world);
        drain_client_events(world, cid, Duration::from_secs(2));
        if let Some(stream) = world._mc_live_streams.remove(&cid) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
    }
    if let Some(socket) = world._mc_live_socket.take() {
        let _ = std::fs::remove_file(&socket);
    }
    if let Some(handle) = world._mc_live_handle.take() {
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            if handle.is_finished() {
                let exit = handle.join().unwrap_or(1);
                world.mc_exit_code = Some(exit);
                world.uds_exit_code = Some(exit);
                break;
            }
            if Instant::now() > deadline {
                world.mc_exit_code = Some(0);
                world.uds_exit_code = Some(0);
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    } else if world.mc_exit_code.is_none() {
        world.mc_exit_code = Some(0);
        world.uds_exit_code = Some(0);
    }
}

// ─── Hook: fix delay + text response stacking for busy scenarios ──────────────
//
// Feature order:
//   And the mock LLM will delay its response by 3 seconds
//   And the mock LLM returns a text response "..."
// The second step currently replaces the server and drops the delay.
// We intercept by storing the delay, then composing sequential responses when
// the text step runs after a delay was requested for multi-client busy tests.
//
// Because we cannot easily wrap the existing given step, busy scenarios that
// need (fast first, delayed second) remount in `when_wait_first_turn` using
// the expected body from the last text response given.

/// Capture expected body when the standard text response given runs after delay.
/// Called from a parallel given that features can use — also patch via world flag.
#[given(
    expr = "the mock LLM returns a text response {string} with prior-turn delay of {int} seconds"
)]
fn given_text_with_prior_delay(world: &mut QuectoWorld, content: String, delay_secs: u64) {
    // First response fast (completed turn), second delayed (busy second turn).
    world._bounded_expected_body = Some(content.clone());
    world._bounded_delay_secs = Some(delay_secs);
    mount_sequential_text_responses(
        world,
        &[
            (content.as_str(), Duration::ZERO),
            (content.as_str(), Duration::from_secs(delay_secs)),
        ],
    );
}

/// After standard delay+text steps, remount sequential responses for busy flow.
/// Invoked from wait-for-first-turn when we detect delay was set.
fn remount_busy_mock_if_needed(world: &mut QuectoWorld) {
    if let (Some(delay), Some(body)) = (
        world._bounded_delay_secs,
        world._bounded_expected_body.clone(),
    ) {
        mount_sequential_text_responses(
            world,
            &[
                (body.as_str(), Duration::ZERO),
                (body.as_str(), Duration::from_secs(delay)),
            ],
        );
    }
}
