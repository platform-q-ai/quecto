//! UDS framing / version-negotiation steps (ADR-0008 part 1, #1059).
//!
//! Scenarios pin protocol behaviour, not byte layout (ADR-0011, retaining the
//! principle from superseded ADR-0010): the scripted
//! client's bytes are authored through `quecto_line_io`'s PRODUCTION frame
//! writer, and the agent's framed replies are decoded through the production
//! frame reader — steps author plain JSON and encode only at this transport
//! boundary.

use super::*;

use quecto::interface::cli::uds_wire::PROTOCOL_ANNOUNCE_PREFIX;
use quecto_line_io::PROTOCOL_FRAME_CAP_BYTES;

/// Which wire framing the scripted UDS client speaks (#1059).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdsWireClient {
    /// Length-prefixed frames (protocol v2).
    Framed,
    /// Legacy `\n`-terminated NDJSON lines (deprecation window).
    Legacy,
    /// Bytes that are neither framing (version-mismatch probe).
    Raw,
}

/// Command-list marker for "send a frame declaring a size above the cap".
/// A `\0` prefix can never collide with an authored JSON command.
const OVER_LIMIT_MARKER: &str = "\0over-limit-frame";

/// Bytes that open with neither a frame prefix (`0x00`) nor legacy JSON (`{`).
const RAW_GARBAGE: &[u8] = &[0xFF, 0x01, 0x02, 0x03];

/// Drive the always-ready `Vec<u8>` / byte-slice transport through the
/// production async frame writer/reader. No reactor is needed (the IO can
/// never return `Pending`), and no nested executor is entered (cucumber's own
/// `LocalPool` rejects a nested `block_on`).
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    let mut f = std::pin::pin!(f);
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    match f.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(v) => v,
        std::task::Poll::Pending => {
            unreachable!("in-memory frame transport must complete synchronously")
        }
    }
}

/// Build the scripted client's wire bytes from the accumulated JSON commands,
/// in the scenario's negotiated framing. Legacy (and scenarios that never
/// picked a framing) newline-join, matching the historical client.
pub fn build_wire_client_bytes(world: &QuectoWorld) -> Vec<u8> {
    match world.uds_wire_client {
        None | Some(UdsWireClient::Legacy) => world
            .uds_commands
            .iter()
            .flat_map(|l| format!("{l}\n").into_bytes())
            .collect(),
        Some(UdsWireClient::Raw) => RAW_GARBAGE.to_vec(),
        Some(UdsWireClient::Framed) => {
            let mut wire = Vec::new();
            block_on(async {
                for cmd in &world.uds_commands {
                    if cmd == OVER_LIMIT_MARKER {
                        // Author the over-cap frame against the REAL writer at
                        // a larger cap (ADR-0011 pattern), so the declared
                        // size — not any hand-rolled byte layout — is what
                        // exceeds the agent's limit.
                        let oversized = vec![b'x'; PROTOCOL_FRAME_CAP_BYTES + 1];
                        quecto_line_io::write_frame(
                            &mut wire,
                            &oversized,
                            PROTOCOL_FRAME_CAP_BYTES + 1,
                        )
                        .await
                        .expect("authoring the over-limit frame must succeed");
                        continue;
                    }
                    quecto_line_io::write_frame(
                        &mut wire,
                        cmd.as_bytes(),
                        PROTOCOL_FRAME_CAP_BYTES,
                    )
                    .await
                    .expect("authoring a framed command must succeed");
                }
            });
            wire
        }
    }
}

/// Decode the agent's reply bytes into event "lines". A framed client's
/// replies are decoded through the production frame reader (pinning that the
/// agent really answered in frames); other clients read newline-delimited
/// text as before.
pub fn parse_wire_events(world: &QuectoWorld, response_bytes: &[u8]) -> Vec<String> {
    match world.uds_wire_client {
        Some(UdsWireClient::Framed) => {
            let mut events = Vec::new();
            block_on(async {
                let mut reader = tokio::io::BufReader::new(response_bytes);
                loop {
                    match quecto_line_io::read_frame_or_legacy_line(
                        &mut reader,
                        PROTOCOL_FRAME_CAP_BYTES,
                    )
                    .await
                    {
                        Ok(Some(quecto_line_io::Incoming::Frame(payload)))
                        | Ok(Some(quecto_line_io::Incoming::LegacyLine(payload))) => {
                            let text = String::from_utf8_lossy(&payload).into_owned();
                            if !text.is_empty() {
                                events.push(text);
                            }
                        }
                        Ok(None) => break,
                        Err(e) => panic!(
                            "a framed client's replies must decode as frames or the legacy \
                             connect-time prelude during negotiation: {e}"
                        ),
                    }
                }
            });
            events
        }
        _ => String::from_utf8_lossy(response_bytes)
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect(),
    }
}

/// Run the deferred UDS execution when this scenario scripted a wire client.
/// Shared `Then` steps call this so framing scenarios don't need an explicit
/// connection-close trigger step. Idempotent (like `execute_uds`).
pub fn ensure_wire_client_executed(world: &mut QuectoWorld) {
    if world.uds_wire_client.is_some() {
        uds_steps::execute_uds(world);
    }
}

fn find_error_event(events: &[String], needle: &str) -> Option<serde_json::Value> {
    events.iter().find_map(|line| {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        let error = v["error"].as_str().unwrap_or_default();
        error.contains(needle).then_some(v)
    })
}

// ─── Given steps ─────────────────────────────────────────────────────────────

#[given("the UDS agent is running with no session")]
fn given_uds_agent_running_no_session(world: &mut QuectoWorld) {
    world.session_name = None;
    world.no_session = true;
}

#[given("a length-prefixed framing client that disconnects after sending")]
fn given_framed_client(world: &mut QuectoWorld) {
    world.uds_wire_client = Some(UdsWireClient::Framed);
}

#[given("a legacy newline-framing client that disconnects after sending")]
fn given_legacy_client(world: &mut QuectoWorld) {
    world.uds_wire_client = Some(UdsWireClient::Legacy);
}

#[given("a raw client that disconnects after sending")]
fn given_raw_client(world: &mut QuectoWorld) {
    world.uds_wire_client = Some(UdsWireClient::Raw);
}

#[given("the client has sent a frame declaring a size above the frame size limit")]
fn given_over_limit_frame_sent(world: &mut QuectoWorld) {
    assert_eq!(
        world.uds_wire_client,
        Some(UdsWireClient::Framed),
        "an over-limit frame needs the length-prefixed framing client"
    );
    world.uds_commands.push(OVER_LIMIT_MARKER.to_string());
}

// ─── When steps ──────────────────────────────────────────────────────────────

#[when(expr = "I send prompt {string} as a length-prefixed frame")]
fn when_send_prompt_framed(world: &mut QuectoWorld, message: String) {
    assert_eq!(
        world.uds_wire_client,
        Some(UdsWireClient::Framed),
        "framed prompts need the length-prefixed framing client"
    );
    let cmd = serde_json::json!({"type": "prompt", "message": message});
    world.uds_commands.push(cmd.to_string());
}

#[when(expr = "I send prompt {string} as a legacy newline-framed line")]
fn when_send_prompt_legacy(world: &mut QuectoWorld, message: String) {
    assert_eq!(
        world.uds_wire_client,
        Some(UdsWireClient::Legacy),
        "legacy prompts need the newline-framing client"
    );
    let cmd = serde_json::json!({"type": "prompt", "message": message});
    world.uds_commands.push(cmd.to_string());
}

#[when("the client sends bytes that are neither a frame nor legacy JSON")]
fn when_send_raw_garbage(world: &mut QuectoWorld) {
    assert_eq!(
        world.uds_wire_client,
        Some(UdsWireClient::Raw),
        "garbage bytes need the raw client"
    );
    // The bytes themselves are fixed by the raw client (`RAW_GARBAGE`); this
    // step is the scenario's single triggering action.
}

#[when("I read the socket announcement")]
fn when_read_socket_announcement(world: &mut QuectoWorld) {
    uds_steps::execute_uds(world);
}

// ─── Then steps ──────────────────────────────────────────────────────────────

#[then("the socket announcement should include a protocol version token")]
fn then_announcement_has_protocol_version(world: &mut QuectoWorld) {
    ensure_wire_client_executed(world);
    let version = world
        .agent_stderr
        .lines()
        .find_map(|l| l.strip_prefix(PROTOCOL_ANNOUNCE_PREFIX))
        .unwrap_or_else(|| {
            panic!(
                "expected a `{PROTOCOL_ANNOUNCE_PREFIX}<version>` line in the socket \
                 announcement\nstderr: {}",
                world.agent_stderr
            )
        });
    let parsed: u8 = version
        .trim()
        .parse()
        .expect("the protocol version token must be numeric so a client can compare it");
    assert!(
        parsed >= 2,
        "length-prefixed framing is protocol v2+; announced {parsed}"
    );
}

#[then("the agent should log a protocol error for the over-limit frame")]
fn then_protocol_error_for_over_limit(world: &mut QuectoWorld) {
    ensure_wire_client_executed(world);
    let event = find_error_event(&world.agent_events, "frame cap").unwrap_or_else(|| {
        panic!(
            "expected a protocol error naming the frame cap\nevents: {:#?}",
            world.agent_events
        )
    });
    assert_eq!(event["command"], "protocol_error", "event: {event}");
    let error = event["error"].as_str().unwrap_or_default();
    assert!(
        error.contains(&(PROTOCOL_FRAME_CAP_BYTES + 1).to_string())
            && error.contains(&PROTOCOL_FRAME_CAP_BYTES.to_string()),
        "the protocol error must name the declared size and the cap; got: {error}"
    );
}

#[then("the agent should log an explicit protocol version-mismatch error")]
fn then_protocol_version_mismatch(world: &mut QuectoWorld) {
    ensure_wire_client_executed(world);
    let event = find_error_event(&world.agent_events, "version mismatch").unwrap_or_else(|| {
        panic!(
            "expected an explicit protocol version-mismatch error\nevents: {:#?}",
            world.agent_events
        )
    });
    assert_eq!(event["command"], "protocol_error", "event: {event}");
}
