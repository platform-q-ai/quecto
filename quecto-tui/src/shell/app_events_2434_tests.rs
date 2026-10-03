//! #2434 review round 1 (L1): a run that recorded no reply (the model
//! answered tool results with nothing, or the tool iteration limit) ends
//! with its tool calls and results as its refs and no reply text on
//! `turn_end` and `agent_end`. There is nothing to rebuild, so neither end
//! event asks for its messages again, on the master or a sub-agent.

use crate::protocol::client::Event;
use crate::shell::app::tui_harness::{TuiHarness, subagent, subagents_changed};

const CALL: &str = "aaaaaaaa-0000-0000-0000-000000000001";
const RESULT: &str = "aaaaaaaa-0000-0000-0000-000000000002";

fn is_get_message(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .is_ok_and(|v| v.get("type").and_then(|t| t.as_str()) == Some("get_message"))
}

/// One tool, then the run's end events with `content_length` as the reply
/// text's length.
fn run_events(content_length: u64) -> Vec<Event> {
    let agent_end = serde_json::json!({
        "type": "agent_end", "messages": [], "messageRefs": [CALL, RESULT],
        "contentLength": content_length,
    });
    vec![
        Event::AgentStart,
        Event::ToolExecutionStart {
            tool_call_id: "c1".into(),
            tool_name: "background".into(),
            args: serde_json::json!({}),
        },
        Event::ToolExecutionEnd {
            tool_call_id: "c1".into(),
            tool_name: "background".into(),
            result: serde_json::json!("started — end your turn now"),
            is_error: false,
        },
        Event::TurnEnd {
            message: serde_json::json!({
                "role": "assistant", "content": "",
                "messageRefs": [CALL, RESULT], "contentLength": content_length,
            }),
        },
        serde_json::from_value(agent_end).expect("agent_end"),
    ]
}

#[tokio::test]
async fn a_master_run_that_recorded_no_reply_is_not_recovered() {
    let mut h = TuiHarness::new().await;
    for event in run_events(0) {
        h.event(event);
    }
    let cmds = h.drain_commands().await;
    assert!(
        !cmds.iter().any(|l| is_get_message(l)),
        "a run with no reply has nothing to rebuild: {cmds:?}"
    );
}

/// Control: reply text the stream never showed is still recovered.
#[tokio::test]
async fn a_master_run_whose_reply_text_is_missing_is_recovered() {
    let mut h = TuiHarness::new().await;
    for event in run_events(12) {
        h.event(event);
    }
    let cmds = h.drain_commands().await;
    assert!(cmds.iter().any(|l| is_get_message(l)), "{cmds:?}");
}

#[tokio::test]
async fn a_sub_agent_run_that_recorded_no_reply_is_not_recovered() {
    let mut h = TuiHarness::new().await;
    h.event(Event::AgentStart);
    h.event(subagents_changed(vec![subagent(
        "worker",
        "running",
        Some(("active", 1, 3)),
    )]));
    for event in run_events(0) {
        h.route("worker", event);
    }
    let cmds = h.drain_commands().await;
    assert!(
        !cmds.iter().any(|l| is_get_message(l)),
        "a child run with no reply has nothing to rebuild: {cmds:?}"
    );
}
