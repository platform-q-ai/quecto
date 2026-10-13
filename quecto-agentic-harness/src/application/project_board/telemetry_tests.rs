//! Epic #2481 T1: a recording exists only while the event log is on; T3:
//! a request's op reads or writes, and names its task and pull request.
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
        listing: None,
        cancelled: false,
        build_us: None,
        result_bytes: 0,
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

/// One request of every kind, each naming task `a` where it names one.
fn every_request() -> Vec<BoardRequest> {
    use super::super::dto::{DraftUpdate, NewBoardTask, NewBoardTaskStatus};
    use super::super::operations::ListFilter;
    use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
    let a = Slug::parse("a").unwrap();
    let change = |op| BoardRequest::Change(a.clone(), op);
    vec![
        BoardRequest::Bootstrap,
        BoardRequest::List(ListFilter::Children(a.clone())),
        BoardRequest::ListReady,
        BoardRequest::ListTakeable,
        BoardRequest::Show(a.clone()),
        BoardRequest::Create(NewBoardTask {
            id: a.clone(),
            title: "A".into(),
            kind: TaskKind::Task,
            description: String::new(),
            status: NewBoardTaskStatus::Draft,
            parent: None,
            depends_on: vec![],
        }),
        BoardRequest::UpdateDraft(DraftUpdate {
            id: a.clone(),
            title: Some("B".into()),
            description: None,
            parent: None,
            depends_on: None,
            expected_updated: None,
        }),
        change(BoardOp::Move(TaskStatus::Ready)),
        change(BoardOp::Renew),
        change(BoardOp::TakeOver),
        change(BoardOp::AddPr(7)),
        change(BoardOp::RemovePr(8)),
        BoardRequest::Complete(a.clone()),
        BoardRequest::Clear {
            path: "tasks/a.md".into(),
            object: "0".repeat(40),
            repeated: Default::default(),
        },
    ]
}

/// T3: an op's access is its request's, so a request refused before it
/// was read records the access it would have had.
#[test]
fn every_op_reads_or_writes_as_its_request_does() {
    let requests = every_request();
    let kinds: Vec<OpKind> = requests.iter().map(op_kind).collect();
    let named: Vec<OpKind> = OpKind::ALL
        .iter()
        .copied()
        .filter(|kind| *kind != OpKind::Unknown)
        .collect();
    assert_eq!(kinds, named, "one request of every op");
    for request in &requests {
        let access = match request.writes() {
            true => Access::Write,
            false => Access::Read,
        };
        assert_eq!(access_of(op_kind(request)), access, "{request:?}");
    }
    assert_eq!(access_of(OpKind::Unknown), Access::Read);
}

/// T3: a record names the task an op's `task` field names, and the pull
/// request it links or unlinks; never a listing's filter.
#[test]
fn a_request_names_its_task_and_pull_request() {
    let requests = every_request();
    let named: Vec<(OpKind, Option<&str>, Option<u64>)> = requests
        .iter()
        .map(|request| {
            (
                op_kind(request),
                task_of(request).map(Slug::as_str),
                pr_of(request),
            )
        })
        .collect();
    let task = |op| (op, Some("a"), None);
    let none = |op| (op, None, None);
    assert_eq!(
        named,
        vec![
            none(OpKind::Bootstrap),
            none(OpKind::List),
            none(OpKind::ListReady),
            none(OpKind::ListTakeable),
            task(OpKind::Show),
            task(OpKind::Create),
            task(OpKind::UpdateDraft),
            task(OpKind::Move),
            task(OpKind::Renew),
            task(OpKind::TakeOver),
            (OpKind::LinkPr, Some("a"), Some(7)),
            (OpKind::UnlinkPr, Some("a"), Some(8)),
            task(OpKind::Complete),
            none(OpKind::ClearSetAside),
        ]
    );
}
