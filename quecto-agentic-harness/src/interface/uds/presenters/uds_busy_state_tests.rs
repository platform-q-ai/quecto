//! #2708 live trial: when the reader answers a guarded harness's own
//! `get_state`, and when it leaves it to the dispatch loop.
use super::{BusyState, intercept};

fn source(busy: bool) -> BusyState {
    let session = crate::composition::bootstrap::agent_turn::build_turn_session("stub".into());
    BusyState {
        busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(busy)),
        state: std::sync::Arc::new(tokio::sync::RwLock::new(
            session.state_snapshot("cli:busy", 0, None, 0, None),
        )),
        runtime_store: Default::default(),
        workflow: None,
        execution: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
    }
}

/// Whether `line` is answered here and, if so, the answer.
async fn ask(
    line: &str,
    source: Option<&BusyState>,
    queued_ahead: bool,
) -> (bool, Option<serde_json::Value>) {
    let clients = crate::interface::uds::controllers::uds_ext_protocol::new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
    crate::interface::uds::controllers::uds_ext_protocol::register_client_writer(&clients, 1, tx);
    let handled = intercept(line, (source, queued_ahead), &clients, 1).await;
    let answer = rx
        .try_recv()
        .ok()
        .map(|line| serde_json::from_str(&line).unwrap());
    (handled, answer)
}

const GET_STATE: &str = r#"{"type":"get_state","id":"s"}"#;

#[tokio::test]
async fn issue_2708_busy_state_answers_only_a_busy_or_queued_guarded_loop() {
    let (idle, busy) = (source(false), source(true));
    for (source, queued_ahead, answered) in [
        (Some(&busy), false, true),
        (Some(&idle), true, true),
        (Some(&idle), false, false),
        (None, true, false),
    ] {
        let (handled, answer) = ask(GET_STATE, source, queued_ahead).await;
        assert_eq!(
            handled,
            answered,
            "busy {:?} queued {queued_ahead}",
            source.map(|s| s.busy.clone())
        );
        assert_eq!(answer.is_some(), answered);
        if let Some(answer) = answer {
            assert_eq!(answer["id"], "s");
            assert_eq!(answer["command"], "get_state");
            assert_eq!(answer["data"]["sessionKey"], "cli:busy");
        }
    }
}

#[tokio::test]
async fn issue_2708_busy_state_leaves_other_lines_and_child_reads_to_the_loop() {
    let busy = source(true);
    for line in [
        r#"{"type":"get_state","id":"c","agent_id":"child"}"#,
        r#"{"type":"get_messages","id":"m"}"#,
        r#"{"type":"prompt","id":"p","message":"get_state"}"#,
        "not json get_state",
    ] {
        assert_eq!(ask(line, Some(&busy), true).await, (false, None), "{line}");
    }
}
