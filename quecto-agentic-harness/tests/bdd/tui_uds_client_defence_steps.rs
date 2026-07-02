//! Step definitions for `tui_uds_client_defence.feature` (#982).
//!
//! These exercise the TUI UDS client's defensive wire-contract in terms of the
//! events the TUI receives and the resource allowance it gives an untrusted
//! event frame.

use super::*;
use quecto_tui::infrastructure::client::{Client, Event};

const MAX_LINE_BYTES: usize = 1_048_576;
use tokio::io::AsyncWriteExt;
use tokio::net::UnixListener;

#[derive(Debug)]
pub struct TuiDefenceStream {
    runtime: tokio::runtime::Runtime,
    socket_path: PathBuf,
    listener: Option<UnixListener>,
    latest_event: Option<Event>,
    completion_agent_end: Option<Event>,
    completion_turn_end: Option<Event>,
    expected_under_cap_token_len: Option<usize>,
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
        latest_event: None,
        completion_agent_end: None,
        completion_turn_end: None,
        expected_under_cap_token_len: None,
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
    let (mut client, _) = rt.block_on(async move {
        let client_future = Client::connect(&socket_path);
        let server = async move {
            let (mut socket, _) = listener.accept().await.expect("server accepts");
            for frame in frames {
                socket.write_all(&frame).await.expect("write frame");
            }
            socket.flush().await.expect("flush");
        };
        let (client, _) = tokio::join!(client_future, server);
        (client.expect("client connects"), ())
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

#[when(
    "the agent sends an event larger than the supported event size followed by a valid token event"
)]
fn agent_sends_oversized_then_valid(world: &mut QuectoWorld) {
    let oversized = [vec![b'x'; MAX_LINE_BYTES + 65_536], b"\n".to_vec()].concat();
    let valid = br#"{"type":"token","token":"later"}
"#
    .to_vec();
    let events = send_frames_and_receive(world, vec![oversized, valid]);
    let stream = world.tui_defence_stream.as_mut().expect("stream");
    stream.latest_event = events.into_iter().next();
}

#[when("the agent sends an event just below the supported event size limit")]
fn agent_sends_event_just_below_limit(world: &mut QuectoWorld) {
    let token_prefix = r#"{"type":"token","token":""#;
    let token_suffix = r#""}"#;
    let token_len = MAX_LINE_BYTES - token_prefix.len() - token_suffix.len() - 1;
    let mut frame = String::with_capacity(MAX_LINE_BYTES);
    frame.push_str(token_prefix);
    frame.push_str(&"a".repeat(token_len));
    frame.push_str(token_suffix);
    assert_eq!(frame.len(), MAX_LINE_BYTES - 1);
    frame.push('\n');
    let events = send_frames_and_receive(world, vec![frame.into_bytes()]);
    let stream = world.tui_defence_stream.as_mut().expect("stream");
    stream.latest_event = events.into_iter().next();
    stream.expected_under_cap_token_len = Some(token_len);
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
fn tui_ignores_oversized_event(world: &mut QuectoWorld) {
    let stream = world.tui_defence_stream.as_ref().expect("stream");
    assert!(
        !matches!(stream.latest_event, Some(Event::Unknown)),
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

#[then("the TUI should receive the event")]
fn tui_receives_event(world: &mut QuectoWorld) {
    let stream = world.tui_defence_stream.as_ref().expect("stream");
    match &stream.latest_event {
        Some(Event::Token { token }) => assert_eq!(
            Some(token.len()),
            stream.expected_under_cap_token_len,
            "just-below-limit token should be delivered intact"
        ),
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
