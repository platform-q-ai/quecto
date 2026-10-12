//! Epic #2481 T1: a recording exists only while the event log is on.
use std::sync::Mutex;

use super::*;
use crate::domain::project_board::value_objects::telemetry::{Access, Client, FrontEnd, OpOutcome};

#[derive(Default)]
struct Log(Mutex<Vec<BoardOpRecord>>);

impl ProjectBoardOpLog for Log {
    fn record(&self, record: BoardOpRecord) {
        self.0.lock().unwrap().push(record);
    }
}

fn record() -> BoardOpRecord {
    BoardOpRecord {
        op: OpKind::Show,
        front_end: FrontEnd::Tool,
        client: Client::Agent,
        access: Access::Read,
        outcome: OpOutcome::Ok,
        fault: None,
        task: None,
        pr: None,
        head: None,
        duration_us: 1,
    }
}

/// Owner condition (2026-10-12): off builds nothing, so nothing records.
#[test]
fn no_recording_exists_while_the_event_log_is_off() {
    let off = BoardRecording::when(RecordingSwitch::resolved(false), || {
        panic!("a log built while the switch is off")
    });
    assert!(off.is_none());
    assert!(BoardRecording::when(RecordingSwitch::resolved(true), || None).is_none());
}

#[test]
fn a_recording_writes_to_its_log_while_on() {
    let log = Arc::new(Log::default());
    let shared: Arc<dyn ProjectBoardOpLog> = log.clone();
    let recording = BoardRecording::when(RecordingSwitch::resolved(true), || Some(shared))
        .expect("on, with a log");
    recording.record(record());
    assert_eq!(*log.0.lock().unwrap(), vec![record()]);
}
