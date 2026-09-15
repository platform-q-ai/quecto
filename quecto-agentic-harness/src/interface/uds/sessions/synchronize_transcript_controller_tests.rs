use super::{SyncFields, SynchronizeTranscriptController};
use crate::application::sessions::active_session::ActiveSessionState;
use crate::application::sessions::dto::TranscriptSync;
use crate::application::sessions::use_cases::SynchronizeTranscript;
use crate::domain::message::Message;
use crate::domain::session_identity::SessionIdentity;
use std::sync::Arc;

fn controller(published: &[Message]) -> SynchronizeTranscriptController {
    let mut state = ActiveSessionState::new(SessionIdentity::ephemeral());
    state.publish(published);
    let state = Arc::new(tokio::sync::RwLock::new(state));
    SynchronizeTranscriptController::new(Arc::new(SynchronizeTranscript::new(state)))
}

fn messages(n: usize) -> Vec<Message> {
    (0..n).map(|i| Message::user(format!("m{i}"))).collect()
}

#[test]
fn a_parent_local_sync_line_parses_its_correlation_and_numeric_position() {
    let parsed = SyncFields::parse_line(r#"{"type":"sync","id":"s1","epoch":2,"sinceRev":3}"#)
        .expect("a parent-local sync");
    assert_eq!(parsed.request_id.as_deref(), Some("s1"));
    assert_eq!((parsed.epoch, parsed.since_rev), (2, 3));
    let uncorrelated = SyncFields::parse_line(r#"{"type":"sync","epoch":0,"sinceRev":0}"#).unwrap();
    assert_eq!(uncorrelated.request_id, None);
    assert_eq!(uncorrelated.clone(), uncorrelated);
}

#[test]
fn anything_but_a_numeric_parent_local_sync_is_not_this_command() {
    for line in [
        r#"{"type":"sync","epoch":2}"#,
        r#"{"type":"sync","sinceRev":2}"#,
        r#"{"type":"get_state","epoch":2,"sinceRev":3}"#,
        r#"{"type":"sync","agent_id":"child","epoch":2,"sinceRev":3}"#,
        r#"{"type":"sync","epoch":-1,"sinceRev":3}"#,
        r#"{"type":"sync","epoch":"2","sinceRev":3}"#,
        r#"{"type":"sync","epoch":2,"sinceRev":3.5}"#,
        r#"{"epoch":2,"sinceRev":3}"#,
        r#"{"type":7,"epoch":2,"sinceRev":3}"#,
        "[]",
        "not json",
    ] {
        assert_eq!(SyncFields::parse_line(line), None, "{line}");
    }
}

#[tokio::test]
async fn sync_maps_the_wire_position_onto_the_use_case() {
    let published = messages(3);
    let controller = controller(&published);
    let TranscriptSync::Delta(delta) = controller.sync(0, 1, 64, |_| true).await else {
        panic!("the current epoch yields a delta");
    };
    assert_eq!(delta.messages.len(), 2);
    assert_eq!(delta.messages[0].content, "m1");
    let TranscriptSync::Reset(reset) = controller.sync(7, 1, 2, |_| true).await else {
        panic!("another epoch yields a reset");
    };
    assert_eq!(
        reset.page.messages.len(),
        2,
        "the reset window is the caller's"
    );
    assert!(reset.page.has_more_before);
}

#[tokio::test]
async fn the_frame_predicate_reaches_the_use_case() {
    let controller = controller(&messages(4));
    let TranscriptSync::Delta(delta) = controller.sync(0, 1, 64, |m| m.content != "m2").await
    else {
        panic!("a delta");
    };
    assert_eq!(delta.messages.len(), 1);
    assert_eq!(delta.messages[0].content, "m1");
    assert_eq!(delta.next_rev, Some(3));
    assert!(format!("{controller:?}").starts_with("SynchronizeTranscriptController"));
}
