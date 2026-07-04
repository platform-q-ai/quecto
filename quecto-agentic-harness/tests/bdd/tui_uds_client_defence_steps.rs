//! Step definitions for `tui_uds_client_defence.feature` (#982, #1016).
//!
//! These exercise the TUI UDS client's defensive wire-contract in terms of the
//! events the TUI receives and the resource allowance it gives an untrusted
//! event frame.

use super::*;
use quecto_tui::infrastructure::client::{Client, Event, MAX_LINE_BYTES};

use tokio::io::AsyncWriteExt;
use tokio::net::UnixListener;

/// Marker embedded in the oversized frame's token payload. The frame is valid
/// JSON, so if the client ever regressed to unbounded reads it would parse and
/// deliver a token containing this marker — which the ignore step rejects.
const OVERSIZED_MARKER: &str = "oversized-payload-";

#[derive(Debug)]
pub struct TuiDefenceStream {
    runtime: tokio::runtime::Runtime,
    socket_path: PathBuf,
    listener: Option<UnixListener>,
    received_events: Vec<Event>,
    latest_event: Option<Event>,
    completion_agent_end: Option<Event>,
    completion_turn_end: Option<Event>,
    expected_under_cap_token_len: Option<usize>,
    expected_under_cap_token: Option<String>,
    capacity_after_oversized_events: Option<usize>,
    capacity_for_later_event: Option<usize>,
    _temp_dir: TempDir,
}

#[given("the TUI is connected to an agent event stream")]
fn tui_connected_to_agent_event_stream(world: &mut QuectoWorld) {
    let temp_dir = tempfile::tempdir().expect("tempdir");
    let socket_path = temp_dir.path().join("agent.sock");
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let listener = runtime
        .block_on(async { UnixListener::bind(&socket_path) })
        .expect("bind socket");
    world.tui_defence_stream = Some(TuiDefenceStream {
        runtime,
        socket_path,
        listener: Some(listener),
        received_events: Vec::new(),
        latest_event: None,
        completion_agent_end: None,
        completion_turn_end: None,
        expected_under_cap_token_len: None,
        expected_under_cap_token: None,
        capacity_after_oversized_events: None,
        capacity_for_later_event: None,
        _temp_dir: temp_dir,
    });
}

fn send_frames_and_receive(world: &mut QuectoWorld, frames: Vec<Vec<u8>>) -> Vec<Event> {
    let stream = world
        .tui_defence_stream
        .as_mut()
        .expect("TUI defence stream");
    let socket_path = stream.socket_path.clone();
    let listener = stream.listener.take().expect("listener");
    let rt = &stream.runtime;
    let mut client = rt.block_on(async move {
        let client_future = Client::connect(&socket_path);
        let server = async move {
            let (mut socket, _) = listener.accept().await.expect("server accepts");
            for frame in frames {
                socket.write_all(&frame).await.expect("write frame");
            }
            socket.flush().await.expect("flush");
        };
        let (client, _) = tokio::join!(client_future, server);
        client.expect("client connects")
    });
    let mut events = Vec::new();
    loop {
        let next = rt.block_on(async {
            tokio::time::timeout(std::time::Duration::from_millis(200), client.recv()).await
        });
        match next {
            Ok(Some(event)) => events.push(event),
            _ => break,
        }
    }
    drop(client);
    events
}

fn read_frames_with_buffer_capacity_observations(
    world: &mut QuectoWorld,
    frames: Vec<Vec<u8>>,
) -> (Vec<Event>, usize, usize) {
    let stream = world
        .tui_defence_stream
        .as_mut()
        .expect("TUI defence stream");
    let socket_path = stream.socket_path.clone();
    let listener = stream.listener.take().expect("listener");
    let rt = &stream.runtime;

    rt.block_on(async move {
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("server accepts");
            for frame in frames {
                socket.write_all(&frame).await.expect("write frame");
            }
            socket.flush().await.expect("flush");
        });
        let socket = tokio::net::UnixStream::connect(&socket_path)
            .await
            .expect("client connects");
        let mut reader = tokio::io::BufReader::new(socket);
        let mut line = Vec::new();
        let mut events = Vec::new();
        let mut capacity_after_oversized_events = 0;
        let mut capacity_for_later_event = 0;

        while let Some(read) =
            quecto_line_io::read_bounded_line_into(&mut reader, &mut line, MAX_LINE_BYTES)
                .await
                .expect("read line")
        {
            if read.truncated || (line.len() >= MAX_LINE_BYTES && !line.ends_with(b"\n")) {
                capacity_after_oversized_events = line.capacity();
                continue;
            }

            capacity_for_later_event = line.capacity();
            if let Ok(text) = std::str::from_utf8(&line) {
                if let Ok(event) = serde_json::from_str::<Event>(text.trim()) {
                    events.push(event);
                }
            }
        }

        server.await.expect("server completes");

        (
            events,
            capacity_after_oversized_events,
            capacity_for_later_event,
        )
    })
}

#[when(
    "the agent sends an event larger than the supported event size followed by a valid token event"
)]
fn agent_sends_oversized_then_valid(world: &mut QuectoWorld) {
    // A well-formed token event whose total frame size exceeds the cap. Were
    // the client's bound ever removed, this would parse and be delivered as a
    // token carrying OVERSIZED_MARKER, which the ignore step would catch.
    let mut oversized = String::with_capacity(MAX_LINE_BYTES + 131_072);
    oversized.push_str(r#"{"type":"token","token":""#);
    oversized.push_str(OVERSIZED_MARKER);
    oversized.push_str(&"x".repeat(MAX_LINE_BYTES + 65_536));
    oversized.push_str("\"}\n");
    let valid = br#"{"type":"token","token":"later"}
"#
    .to_vec();
    let events = send_frames_and_receive(world, vec![oversized.into_bytes(), valid]);
    let stream = world.tui_defence_stream.as_mut().expect("stream");
    stream.latest_event = events.first().cloned();
    stream.received_events = events;
}

#[when("the agent sends an event just below the supported event size limit")]
fn agent_sends_event_just_below_limit(world: &mut QuectoWorld) {
    let token_prefix = r#"{"type":"token","token":""#;
    let token_suffix = r#""}"#;
    let token_len = MAX_LINE_BYTES - token_prefix.len() - token_suffix.len() - 1;
    let token: String = (0..token_len)
        .map(|idx| char::from(b'a' + (idx % 26) as u8))
        .collect();
    let mut frame = String::with_capacity(MAX_LINE_BYTES);
    frame.push_str(token_prefix);
    frame.push_str(&token);
    frame.push_str(token_suffix);
    assert_eq!(frame.len(), MAX_LINE_BYTES - 1);
    frame.push('\n');
    let events = send_frames_and_receive(world, vec![frame.into_bytes()]);
    let stream = world.tui_defence_stream.as_mut().expect("stream");
    stream.latest_event = events.into_iter().next();
    stream.expected_under_cap_token_len = Some(token_len);
    stream.expected_under_cap_token = Some(token);
}

#[when("the agent sends repeated oversized events followed by a valid token event")]
fn agent_sends_repeated_oversized_events_then_valid(world: &mut QuectoWorld) {
    let frames = (0..3)
        .map(|idx| {
            let mut oversized = String::with_capacity(MAX_LINE_BYTES + 131_072);
            oversized.push_str(r#"{"type":"token","token":""#);
            oversized.push_str(OVERSIZED_MARKER);
            oversized.push_str(&idx.to_string());
            oversized.push_str(&"x".repeat(MAX_LINE_BYTES + 65_536));
            oversized.push_str("\"}\n");
            oversized.into_bytes()
        })
        .chain(std::iter::once(
            b"{\"type\":\"token\",\"token\":\"later\"}\n".to_vec(),
        ))
        .collect();
    let (events, capacity_after_oversized_events, capacity_for_later_event) =
        read_frames_with_buffer_capacity_observations(world, frames);
    let stream = world.tui_defence_stream.as_mut().expect("stream");
    stream.latest_event = events.first().cloned();
    stream.received_events = events;
    stream.capacity_after_oversized_events = Some(capacity_after_oversized_events);
    stream.capacity_for_later_event = Some(capacity_for_later_event);
}

#[when("the agent reports completion with details the TUI does not display")]
fn agent_reports_completion_with_undisplayed_details(world: &mut QuectoWorld) {
    let events = send_frames_and_receive(
        world,
        vec![
            br#"{"type":"agent_end","messages":[{"role":"assistant","content":"undisplayed-agent-detail"}]}
"#
            .to_vec(),
            br#"{"type":"turn_end","message":{"role":"assistant","content":"visible","contextTokens":7,"maxContextTokens":10},"toolResults":[{"content":[{"type":"text","text":"undisplayed-tool-detail"}]}]}
"#
            .to_vec(),
        ],
    );
    let stream = world.tui_defence_stream.as_mut().expect("stream");
    for event in events {
        match event {
            Event::AgentEnd => stream.completion_agent_end = Some(Event::AgentEnd),
            Event::TurnEnd { .. } => stream.completion_turn_end = Some(event),
            _ => {}
        }
    }
}

#[then("the TUI should ignore the oversized event")]
#[then("the TUI should ignore the oversized events")]
fn tui_ignores_oversized_event(world: &mut QuectoWorld) {
    let stream = world.tui_defence_stream.as_ref().expect("stream");
    assert!(
        !stream.received_events.iter().any(|event| matches!(
            event,
            Event::Token { token } if token.contains(OVERSIZED_MARKER)
        )),
        "oversized event must not be delivered as an event"
    );
}

#[then("the TUI should receive the later token event")]
fn tui_receives_later_token_event(world: &mut QuectoWorld) {
    let stream = world.tui_defence_stream.as_ref().expect("stream");
    match &stream.latest_event {
        Some(Event::Token { token }) => assert_eq!(token, "later"),
        other => panic!("expected later token event, got {other:?}"),
    }
}

#[then("the TUI should reclaim bounded input buffer capacity for later events")]
fn tui_reclaims_bounded_input_buffer_capacity(world: &mut QuectoWorld) {
    let stream = world.tui_defence_stream.as_ref().expect("stream");
    let capacity_after_oversized_events = stream
        .capacity_after_oversized_events
        .expect("capacity after oversized events observed");
    let capacity_for_later_event = stream
        .capacity_for_later_event
        .expect("capacity for later event observed");

    assert!(
        capacity_after_oversized_events <= MAX_LINE_BYTES + 4096,
        "oversized events must not inflate the reusable input buffer beyond the protocol cap plus a small constant; capacity was {capacity_after_oversized_events}"
    );
    assert!(
        capacity_for_later_event <= 64 * 1024,
        "later events must reclaim reusable input buffer capacity after oversized events; capacity was {capacity_for_later_event}"
    );
}

#[then("the TUI should receive the event")]
fn tui_receives_event(world: &mut QuectoWorld) {
    let stream = world.tui_defence_stream.as_ref().expect("stream");
    match &stream.latest_event {
        Some(Event::Token { token }) => {
            assert_eq!(
                Some(token.len()),
                stream.expected_under_cap_token_len,
                "just-below-limit token should be delivered intact"
            );
            assert_eq!(
                Some(token),
                stream.expected_under_cap_token.as_ref(),
                "just-below-limit token content should be delivered intact"
            );
        }
        other => panic!("expected token event, got {other:?}"),
    }
}

#[then("completion is shown as before")]
fn completion_is_shown_as_before(world: &mut QuectoWorld) {
    let stream = world.tui_defence_stream.as_ref().expect("stream");
    assert!(matches!(stream.completion_agent_end, Some(Event::AgentEnd)));
    match &stream.completion_turn_end {
        Some(Event::TurnEnd { message }) => {
            assert_eq!(message["content"], "visible");
            assert_eq!(message["contextTokens"], 7);
        }
        other => panic!("expected turn_end event, got {other:?}"),
    }
}

#[then("undisplayed completion details do not remain in the client event")]
fn undisplayed_completion_details_are_discarded(world: &mut QuectoWorld) {
    let stream = world.tui_defence_stream.as_ref().expect("stream");
    let debug = format!(
        "{:?}{:?}",
        stream.completion_agent_end, stream.completion_turn_end
    );
    assert!(!debug.contains("undisplayed-agent-detail"));
    assert!(!debug.contains("undisplayed-tool-detail"));
}
