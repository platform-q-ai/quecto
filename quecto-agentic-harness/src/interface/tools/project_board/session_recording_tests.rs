//! Epic #2481 T3: the board tool's calls are recorded as the session's
//! recording is bound, follow a session switch, and stop with it.
use super::*;
use crate::application::tools::ports::Tool;
use crate::interface::tools::project_board::recording::tests::{Records, recording_into};
use crate::interface::tools::project_board::site::BoardSite;
use crate::interface::tools::project_board::tool::BoardTool;

/// A tool over a board that is not composed: every call is refused, and
/// recorded all the same.
fn tool(recordings: &Arc<SessionRecording>) -> BoardTool {
    let site = BoardSite {
        repo: ".".into(),
        remote: "origin".into(),
        build: None,
        launcher: None,
        swarm_session: None,
    };
    BoardTool::recorded_in(site, Arc::clone(recordings))
}

fn ops(records: &Records) -> Vec<&'static str> {
    records
        .taken()
        .iter()
        .map(|record| record.op.as_str())
        .collect()
}

#[tokio::test]
async fn records_follow_a_session_switch_and_stop_with_it() {
    let (first, second) = (Arc::new(Records::default()), Arc::new(Records::default()));
    let recordings = Arc::new(SessionRecording::default());
    let tool = tool(&recordings);
    recordings.bind(Some(recording_into(&first)));
    assert!(recordings.current().is_some());
    tool.execute(r#"{"op": "list"}"#).await.unwrap();
    recordings.follow(|| Some(recording_into(&second)));
    assert!(
        recordings.current().is_some(),
        "a recording follows the switch"
    );
    tool.execute(r#"{"op": "show", "task": "a"}"#)
        .await
        .unwrap();
    assert_eq!((ops(&first), ops(&second)), (vec!["list"], vec!["show"]));
    recordings.stop();
    recordings.follow(|| panic!("a stopped session builds nothing"));
    assert!(
        recordings.current().is_none(),
        "nothing bound, nothing follows"
    );
    tool.execute(r#"{"op": "list"}"#).await.unwrap();
    assert_eq!((ops(&first), ops(&second)), (vec![], vec![]));
}

/// Owner condition (2026-10-12): with the event log off nothing is bound,
/// so no call is recorded, nor does a session switch start one.
#[tokio::test]
async fn nothing_is_recorded_while_the_event_log_is_off() {
    let recordings = Arc::new(SessionRecording::default());
    recordings.bind(None);
    assert!(recordings.current().is_none());
    tool(&recordings)
        .execute(r#"{"op": "list"}"#)
        .await
        .unwrap();
    recordings.follow(|| panic!("an unbound session builds nothing"));
    assert!(recordings.current().is_none());
}

/// A switch whose rebuild finds no log to write to records nothing more.
#[test]
fn a_switch_to_no_log_stops_recording() {
    let records = Arc::new(Records::default());
    let recordings = SessionRecording::default();
    recordings.bind(Some(recording_into(&records)));
    recordings.follow(|| None);
    assert!(recordings.current().is_none());
}

/// Review round 2: a session switch checks and swaps under one write
/// lock, so a `stop` racing the rebuild is never undone.
#[test]
fn a_stop_racing_a_switch_is_never_undone() {
    let records = Arc::new(Records::default());
    let recordings = Arc::new(SessionRecording::default());
    recordings.bind(Some(recording_into(&records)));
    let mut stopper = None;
    recordings.follow(|| {
        let racing = Arc::clone(&recordings);
        stopper = Some(std::thread::spawn(move || racing.stop()));
        std::thread::sleep(std::time::Duration::from_millis(50));
        Some(recording_into(&records))
    });
    stopper.expect("the rebuild ran").join().unwrap();
    assert!(recordings.current().is_none(), "the stop stands");
}
