//! #2708: which `get_state` lines a guarded harness's connection answers
//! itself, and what it answers.
use super::{BusyState, asked};

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

const GET_STATE: &str = r#"{"type":"get_state","id":"s","since":4}"#;

#[test]
fn issue_2708_busy_state_answers_only_a_busy_guarded_loop() {
    let (idle, busy) = (source(false), source(true));
    assert_eq!(
        asked(GET_STATE, Some(&busy)),
        Some((Some("s".into()), Some(4)))
    );
    assert_eq!(asked(GET_STATE, Some(&idle)), None);
    assert_eq!(asked(GET_STATE, None), None);
}

#[test]
fn issue_2708_busy_state_leaves_other_lines_and_child_reads_to_the_loop() {
    let busy = source(true);
    for line in [
        r#"{"type":"get_state","id":"c","agent_id":"child"}"#,
        r#"{"type":"get_messages","id":"m"}"#,
        r#"{"type":"prompt","id":"p","message":"get_state"}"#,
        "not json get_state",
    ] {
        assert_eq!(asked(line, Some(&busy)), None, "{line}");
    }
}

async fn answer(source: &BusyState) -> serde_json::Value {
    let line = source.answer((Some("s".into()), None)).await;
    assert!(line.ends_with('\n'));
    serde_json::from_str(&line).unwrap()
}

/// Review #2762 round 3: between a run's end (its phase already idle) and
/// its `agent_end`, or between back-to-back turns, a busy answer reports a
/// busy phase, never `idle`; once the loop is idle, `idle` it is.
#[tokio::test]
async fn issue_2708_busy_state_a_busy_answer_never_reports_idle() {
    let busy = answer(&source(true)).await;
    assert_eq!(
        (&busy["id"], &busy["command"]),
        (&serde_json::json!("s"), &serde_json::json!("get_state"))
    );
    assert_eq!(busy["data"]["sessionKey"], "cli:busy");
    assert_eq!(busy["data"]["state"], "finalizing", "{busy}");
    assert_eq!(answer(&source(false)).await["data"]["state"], "idle");
}
