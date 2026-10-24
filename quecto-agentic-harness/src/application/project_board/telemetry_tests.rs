//! Epic #2481 T1: a recording exists only while the event log is on; T3:
//! a request's op reads or writes, and names its task and pull request.
use crate::domain::project_board::services::transitions::MoveReason;
use std::sync::Mutex;

use super::*;
use crate::domain::project_board::entities::plan::PlanEntry;
use crate::domain::project_board::value_objects::telemetry::{Access, Client, FrontEnd, OpOutcome};

#[derive(Default)]
struct Log(Mutex<Vec<BoardOpRecord>>);

impl ProjectBoardOpLog for Log {
    fn record(&self, record: BoardOpRecord) {
        self.0.lock().unwrap().push(record);
    }

    fn record_start(
        &self,
        _record: crate::domain::project_board::value_objects::telemetry::BoardStartRecord,
    ) {
    }

    fn record_fold_back(&self, _record: FoldBackRecord) {
        unreachable!("a board_op test writes no fold-back");
    }
}

fn record() -> BoardOpRecord {
    BoardOpRecord {
        op: OpKind::Show,
        front_end: FrontEnd::Tool,
        client: Client::Agent,
        access: Access::Read,
        board: None,
        outcome: OpOutcome::Ok,
        fault: None,
        task: None,
        pr: None,
        head: None,
        listing: None,
        plans: None,
        runs: None,
        plan_version: None,
        items: None,
        roles: None,
        plan_state_from: None,
        plan_state_to: None,
        run: None,
        run_outcome: None,
        status_from: None,
        status_to: None,
        claim: None,
        attempts: None,
        races_lost: None,
        lock_races: None,
        swarm_report: None,
        pause_ms: None,
        completion: None,
        normalised: None,
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
    use super::super::dto::{
        DraftUpdate, NewBoardTask, NewBoardTaskStatus, PlanAcceptance, PlanDiscard, PlanEdit,
        PlanProposal,
    };
    use super::super::operations::ListFilter;
    use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
    let a = Slug::parse("a").unwrap();
    let change = |op| BoardRequest::Change(a.clone(), op);
    vec![
        BoardRequest::Bootstrap,
        BoardRequest::List(ListFilter::Children(a.clone())),
        BoardRequest::ListReady,
        BoardRequest::ListTakeable,
        BoardRequest::Show(a.clone(), super::super::operations::PlanView::default()),
        BoardRequest::Create(NewBoardTask {
            id: a.clone(),
            title: "A".into(),
            kind: TaskKind::Task,
            description: String::new(),
            status: NewBoardTaskStatus::Draft,
            parent: None,
            depends_on: vec![],
            entry: crate::domain::project_board::entities::task_parts::Entry::Sectioned,
        }),
        BoardRequest::UpdateDraft(DraftUpdate {
            id: a.clone(),
            title: Some("B".into()),
            description: None,
            parent: None,
            depends_on: None,
            expected_updated: None,
            entry: crate::domain::project_board::entities::task_parts::Entry::Sectioned,
        }),
        change(BoardOp::Move(TaskStatus::Ready, MoveReason::Absent)),
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
        BoardRequest::ProposePlan(PlanProposal {
            handoff: None,
            entry: PlanEntry::Legacy,
            task: a.clone(),
            approach: String::new(),
            items: vec![],
            team: None,
        }),
        BoardRequest::EditPlan(PlanEdit {
            handoff: None,
            entry: PlanEntry::Legacy,
            task: a.clone(),
            version: 1,
            approach: Some("A".into()),
            items: None,
            team: None,
            expected_updated: None,
        }),
        BoardRequest::AcceptPlan(PlanAcceptance {
            task: a.clone(),
            version: 1,
            expected_updated:
                crate::domain::project_board::value_objects::timestamp::Timestamp::parse(
                    "2026-10-10T09:00:00Z",
                )
                .unwrap(),
            expected_accepted: None,
        }),
        BoardRequest::DiscardPlan(PlanDiscard {
            task: a.clone(),
            version: 1,
            expected_updated: None,
        }),
        BoardRequest::EndRun(crate::application::project_board::dto::RunEnd {
            task: a.clone(),
            run: 1,
            outcome: crate::application::project_board::dto::RunEndOutcome::Lost,
            expected_updated: None,
        }),
        BoardRequest::Start(super::super::dto::TaskStart {
            task: a.clone(),
            plan_version: 1,
        }),
        change(BoardOp::DoneWithoutRun("By hand".into())),
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
            task(OpKind::ProposePlan),
            task(OpKind::EditPlan),
            task(OpKind::AcceptPlan),
            task(OpKind::DiscardPlan),
            task(OpKind::EndRun),
            task(OpKind::Start),
            task(OpKind::DoneWithoutRun),
        ]
    );
}

/// Epic #2481 T4: the use cases' facts as a record holds them.
mod facts {
    use super::*;
    use crate::application::project_board::dto::ProjectBoardError;
    use crate::application::project_board::operations::Completion;
    use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
    use crate::domain::project_board::value_objects::telemetry::{
        ClaimFacts, CompletionFacts, CompletionKind, Normalised, StaticKind, TextChange,
    };
    use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

    fn claim(held_s: i64) -> ClaimFacts {
        ClaimFacts {
            held_s,
            left_s: 60,
            holder_is_writer: true,
        }
    }

    fn completion(kind: CompletionKind) -> CompletionFacts {
        CompletionFacts {
            completion: kind,
            completion_code: None,
            completion_rule: None,
            prs_linked: 1,
            prs_pending: None,
            completion_attempts: None,
            completion_races_lost: None,
            completion_pause_ms: None,
        }
    }

    /// A record whose every T4 field a front end or an earlier fill set.
    fn filled() -> BoardOpRecord {
        BoardOpRecord {
            status_from: Some(StaticKind::new("ready")),
            status_to: Some(StaticKind::new("claimed")),
            claim: Some(claim(1)),
            attempts: Some(1),
            races_lost: Some(0),
            pause_ms: Some(5),
            completion: Some(completion(CompletionKind::Done)),
            normalised: Some(Normalised {
                title: Some(vec![]),
                description: None,
                approach: None,
                items: None,
            }),
            plans: Some(2),
            runs: Some(1),
            ..record()
        }
    }

    #[test]
    fn fill_writes_only_the_facts_it_has() {
        let mut record = filled();
        OpFacts::default().fill(&mut record);
        assert_eq!(record, filled(), "no fact, nothing written over");
        let facts = OpFacts {
            status_from: Some(TaskStatus::InProgress),
            attempts: Some(3),
            lost: Some(2),
            ..OpFacts::default()
        };
        facts.fill(&mut record);
        let want = BoardOpRecord {
            status_from: Some(StaticKind::new("in_progress")),
            attempts: Some(3),
            races_lost: Some(2),
            ..filled()
        };
        assert_eq!(record, want);
    }

    #[test]
    fn fill_writes_every_fact_it_has() {
        let facts = OpFacts {
            status_from: Some(TaskStatus::Review),
            status_to: Some(TaskStatus::Done),
            claim: Some(claim(-3)),
            attempts: Some(2),
            lost: Some(1),
            pause_ms: Some(70),
            completion: Some(completion(CompletionKind::Pending)),
            normalised: Some(Normalised {
                title: None,
                description: Some(vec![TextChange::Crlf]),
                approach: None,
                items: None,
            }),
            plans: Some(4),
            runs: Some(0),
            plan_version: Some(3),
            items: Some(7),
            roles: Some(0),
            plan_state_from: Some(
                crate::domain::project_board::value_objects::vocabulary::PlanState::Draft,
            ),
            plan_state_to: Some(
                crate::domain::project_board::value_objects::vocabulary::PlanState::Discarded,
            ),
            run: Some(2),
            run_outcome: Some(
                crate::domain::project_board::value_objects::vocabulary::RunOutcome::Lost,
            ),
            // #2663: a write's lock races, its commit and its swarm report.
            lock_races: Some(1),
            head: Some(crate::application::project_board::ports::BoardHead(
                "0123456789abcdef0123456789abcdef01234567".into(),
            )),
            swarm_report: Some(
                crate::application::project_board::telemetry::ReportSeen::NotChecked,
            ),
            // A start's and an end's fold-back facts go to their own
            // records, never the board_op's.
            start: Some(Default::default()),
            fold_back: None,
        };
        let mut record = record();
        facts.fill(&mut record);
        let line = serde_json::to_value(&record).unwrap();
        let want = serde_json::json!({
            "op": "show", "front_end": "tool", "client": "agent", "access": "read",
            "outcome": "ok", "result_bytes": 0, "duration_us": 1,
            "status_from": "review", "status_to": "done",
            "claim": {"held_s": -3, "left_s": 60, "holder_is_writer": true},
            "attempts": 2, "races_lost": 1, "lock_races": 1, "pause_ms": 70,
            "head": "0123456789abcdef0123456789abcdef01234567", "swarm_report": "not_checked",
            "completion": "pending", "prs_linked": 1,
            "normalised": {"description": ["crlf"]}, "plans": 4, "runs": 0,
            "plan_version": 3, "items": 7, "roles": 0,
            "plan_state_from": "draft", "plan_state_to": "discarded",
            "run": 2, "run_outcome": "lost",
        });
        assert_eq!(line, want);
    }

    #[test]
    fn a_completion_is_told_by_its_kind_and_counts() {
        let pending = Completion::Pending(vec![8, 9]);
        let failed = Completion::Failed(Box::new(ProjectBoardError::Rule(BoardRuleError::NoPrs)));
        assert_eq!(
            completion_facts_of(&pending, 3),
            CompletionFacts {
                completion: CompletionKind::Pending,
                completion_code: None,
                completion_rule: None,
                prs_linked: 3,
                prs_pending: Some(2),
                completion_attempts: None,
                completion_races_lost: None,
                completion_pause_ms: None,
            }
        );
        assert_eq!(
            completion_facts_of(&failed, 1),
            completion(CompletionKind::Failed)
        );
    }
}

mod fold_back_facts {
    use super::*;
    use crate::application::project_board::dto::FoldBackFacts;
    use crate::domain::project_board::value_objects::code_ref::CommitId;
    use crate::domain::project_board::value_objects::telemetry::{
        ItemCounts, RunOutcomeKind, TaskRef,
    };

    fn bare() -> FoldBackRecord {
        FoldBackRecord {
            task: TaskRef::of(&Slug::parse("board-store").unwrap()),
            run: 1,
            board: None,
            outcome: OpOutcome::Ok,
            run_outcome: None,
            items: None,
            unmatched: None,
            extra: None,
            evidence: None,
            evidence_dropped: None,
            cost_tokens: None,
            attempts: Some(9),
            races_lost: None,
            lock_races: None,
            bytes: None,
            head: None,
            duration_us: 5,
        }
    }

    #[test]
    fn fill_writes_every_fact_it_has_and_keeps_the_rest() {
        let mut record = bare();
        FoldBackFacts::default().fill(&mut record);
        assert_eq!(record, bare());
        let counts = ItemCounts {
            todo: 1,
            doing: 2,
            review: 3,
            done: 4,
            blocked: 5,
        };
        let facts = FoldBackFacts {
            run_outcome: Some(RunOutcomeKind::Stopped),
            items: Some(counts),
            unmatched: Some(1),
            extra: Some(2),
            evidence: Some(3),
            evidence_dropped: Some(4),
            cost_tokens: Some(5),
            attempts: Some(6),
            lost: Some(7),
            lock_races: Some(9),
            bytes: Some(8),
            head: CommitId::parse("0123456789abcdef0123456789abcdef01234567").ok(),
        };
        facts.fill(&mut record);
        let expected = FoldBackRecord {
            run_outcome: Some(RunOutcomeKind::Stopped),
            items: Some(counts),
            unmatched: Some(1),
            extra: Some(2),
            evidence: Some(3),
            evidence_dropped: Some(4),
            cost_tokens: Some(5),
            attempts: Some(6),
            races_lost: Some(7),
            lock_races: Some(9),
            bytes: Some(8),
            head: CommitId::parse("0123456789abcdef0123456789abcdef01234567").ok(),
            ..bare()
        };
        assert_eq!(record, expected);
    }

    #[test]
    fn a_count_is_held_whole_or_saturated() {
        assert_eq!(counted(7), 7);
        assert_eq!(
            counted(usize::MAX),
            u64::try_from(usize::MAX).unwrap_or(u64::MAX)
        );
    }
}
