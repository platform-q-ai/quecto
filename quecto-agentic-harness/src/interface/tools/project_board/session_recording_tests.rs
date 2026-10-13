//! Epic #2481 T3: the board tool's calls are recorded in the session's
//! event log while it is on, follow a session switch, and stop with it.
use serde_json::Value;

use super::*;
use crate::application::tools::ports::Tool;
use crate::composition::bootstrap::project_board::session_board_recording;
use crate::infrastructure::config::telemetry::EventLogConfig;
use crate::interface::tools::project_board::site::BoardSite;
use crate::interface::tools::project_board::tool::BoardTool;

fn telemetry(enabled: bool) -> TelemetryConfig {
    TelemetryConfig {
        event_log: EventLogConfig { enabled },
    }
}

fn builder() -> SessionRecordingBuilder {
    SessionRecordingBuilder(session_board_recording)
}

/// The `board_op` lines of `session`'s log under `base`.
fn board_ops(base: &Path, session: &str) -> Vec<Value> {
    let text = std::fs::read_to_string(base.join("audit").join(format!("{session}.jsonl")))
        .unwrap_or_default();
    text.lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|line| line["event"] == "board_op")
        .collect()
}

/// A tool over a board that is not composed: every call is refused, and
/// recorded all the same.
fn tool(recordings: &Arc<SessionRecording>) -> BoardTool {
    let site = BoardSite {
        repo: ".".into(),
        remote: "origin".into(),
        build: None,
    };
    BoardTool::recorded_in(site, Arc::clone(recordings))
}

#[tokio::test]
async fn records_follow_a_session_switch_and_stop_with_the_log() {
    let base = tempfile::tempdir().unwrap();
    let (first, second) = (
        AuditLog::open_sync(base.path(), "first").unwrap(),
        AuditLog::open_sync(base.path(), "second").unwrap(),
    );
    let recordings = Arc::new(SessionRecording::default());
    let tool = tool(&recordings);
    assert!(recordings.bind(builder(), &telemetry(true), base.path(), &first));
    tool.execute(r#"{"op": "list"}"#).await.unwrap();
    assert!(recordings.follow(&second), "a recording follows the switch");
    tool.execute(r#"{"op": "show", "task": "a"}"#)
        .await
        .unwrap();
    let ops = |session| -> Vec<Value> {
        board_ops(base.path(), session)
            .iter()
            .map(|line| line["op"].clone())
            .collect()
    };
    assert_eq!(ops("first"), ["list"]);
    assert_eq!(ops("second"), ["show"]);
    let line = &board_ops(base.path(), "second")[0];
    assert_eq!(
        (
            &line["front_end"],
            &line["client"],
            &line["code"],
            &line["turn"]
        ),
        (
            &Value::from("tool"),
            &Value::from("agent"),
            &Value::from("store_unavailable"),
            &Value::Null
        ),
        "{line}"
    );
    recordings.stop();
    assert!(!recordings.follow(&first), "nothing bound, nothing follows");
    tool.execute(r#"{"op": "list"}"#).await.unwrap();
    assert_eq!(ops("first"), ["list"], "a stopped session records nothing");
}

/// Owner condition (2026-10-12): with the event log off, nothing is bound,
/// so no call is recorded, nor does a session switch start one.
#[tokio::test]
async fn nothing_is_recorded_while_the_event_log_is_off() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "off").unwrap();
    let recordings = Arc::new(SessionRecording::default());
    assert!(!recordings.bind(builder(), &telemetry(false), base.path(), &log));
    assert!(recordings.current().is_none());
    tool(&recordings)
        .execute(r#"{"op": "list"}"#)
        .await
        .unwrap();
    assert!(!recordings.follow(&log));
    assert_eq!(board_ops(base.path(), "off"), Vec::<Value>::new());
}
