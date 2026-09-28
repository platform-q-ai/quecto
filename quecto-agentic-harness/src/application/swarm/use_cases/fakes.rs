//! In-memory doubles of the board ports for the use-case and gate tests,
//! in the manner of Python's `MemoryRepository` (`tests/swarm_policy_test.py`):
//! a transaction works on a copy of the state that replaces it only when
//! the work succeeds, and every write and id draw is journalled in order.
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::application::swarm::dto::{
    MemberClaimCounts, MemberRow, NewMember, NewRun, RunContract,
};
use crate::application::swarm::ports::{
    BoardEncoding, BoardEvents, BoardMembers, BoardRepository, BoardRuns, BoardWork, Clock,
    IdSource,
};
use crate::domain::swarm::{BoardError, MemberRecord, MemberState, RunRecord, RunState};

/// One recorded `events` row.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordedEvent {
    pub actor: String,
    pub time: f64,
    pub action: String,
    pub detail: Value,
}

#[derive(Clone, Debug)]
pub struct StoredRun {
    pub id: String,
    pub record: RunRecord,
    pub contract: RunContract,
}

#[derive(Clone, Debug, Default)]
pub struct BoardState {
    pub run: Option<StoredRun>,
    pub members: Vec<MemberRow>,
    pub events: Vec<RecordedEvent>,
}

/// A journal shared by the board and the id source, so a test reads the
/// order of draws and writes across both.
pub type Journal = Arc<Mutex<Vec<String>>>;

#[derive(Default)]
pub struct MemoryBoard {
    pub state: Mutex<BoardState>,
    pub journal: Journal,
    /// `create` of each transaction opened, in order.
    pub transactions: Mutex<Vec<bool>>,
}

impl MemoryBoard {
    pub fn with(state: BoardState) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(state),
            ..Self::default()
        })
    }

    pub fn snapshot(&self) -> BoardState {
        self.state.lock().unwrap().clone()
    }

    pub fn journal(&self) -> Vec<String> {
        self.journal.lock().unwrap().clone()
    }

    pub fn transactions(&self) -> Vec<bool> {
        self.transactions.lock().unwrap().clone()
    }
}

impl BoardRepository for MemoryBoard {
    fn atomic(&self, create: bool, work: &mut BoardWork<'_>) -> Result<(), BoardError> {
        self.transactions.lock().unwrap().push(create);
        let working = RefCell::new(self.snapshot());
        let transaction = MemoryTransaction {
            state: &working,
            journal: &self.journal,
        };
        work(&transaction)?;
        *self.state.lock().unwrap() = working.into_inner();
        Ok(())
    }
}

struct MemoryTransaction<'a> {
    state: &'a RefCell<BoardState>,
    journal: &'a Journal,
}

impl MemoryTransaction<'_> {
    fn note(&self, entry: String) {
        self.journal.lock().unwrap().push(entry);
    }
}

impl BoardRuns for MemoryTransaction<'_> {
    fn run(&self) -> Result<Option<RunRecord>, BoardError> {
        Ok(self
            .state
            .borrow()
            .run
            .as_ref()
            .map(|run| run.record.clone()))
    }

    fn run_id(&self) -> Result<Option<String>, BoardError> {
        Ok(self.state.borrow().run.as_ref().map(|run| run.id.clone()))
    }

    fn insert_run(&self, run: &NewRun) -> Result<(), BoardError> {
        self.note(format!("insert_run {}", run.id));
        let mut state = self.state.borrow_mut();
        assert!(state.run.is_none(), "the board holds one run");
        state.run = Some(StoredRun {
            id: run.id.clone(),
            record: RunRecord {
                status: run.status.clone(),
                coordinator: run.coordinator.clone(),
                deadline: run.contract.deadline,
                member_limit: run.contract.member_limit,
                outcome: None,
                outcome_reason: None,
            },
            contract: run.contract.clone(),
        });
        Ok(())
    }

    fn update_run_contract(&self, contract: &RunContract) -> Result<(), BoardError> {
        self.note("update_run_contract".to_owned());
        let mut state = self.state.borrow_mut();
        let run = state.run.as_mut().expect("a run to update");
        run.record.deadline = contract.deadline;
        run.record.member_limit = contract.member_limit;
        run.record.status = RunState::RUNNING;
        run.contract = contract.clone();
        Ok(())
    }

    fn propose_outcome(&self, outcome: &str, reason: &str) -> Result<(), BoardError> {
        self.note(format!("propose_outcome {outcome}"));
        let mut state = self.state.borrow_mut();
        let run = state.run.as_mut().expect("a run to pause");
        run.record.status = RunState::PAUSED;
        run.record.outcome = Some(outcome.to_owned());
        run.record.outcome_reason = Some(reason.to_owned());
        Ok(())
    }
}

impl BoardMembers for MemoryTransaction<'_> {
    fn member(&self, id: &str) -> Result<Option<MemberRecord>, BoardError> {
        Ok(self
            .state
            .borrow()
            .members
            .iter()
            .find(|row| row.id == id)
            .map(|row| MemberRecord {
                id: row.id.clone(),
                status: MemberState::new(row.status.clone()),
                reservation: row.reservation.clone(),
            }))
    }

    fn members(&self) -> Result<Vec<MemberRow>, BoardError> {
        Ok(self.state.borrow().members.clone())
    }

    fn usage(&self) -> Result<i64, BoardError> {
        Ok(count(&self.state.borrow().members, |status| {
            matches!(status, "live" | "reserved")
        }))
    }

    fn not_dead(&self) -> Result<i64, BoardError> {
        Ok(count(&self.state.borrow().members, |status| {
            status != "dead"
        }))
    }

    fn claim_counts(&self, coordinator: Option<&str>) -> Result<MemberClaimCounts, BoardError> {
        let state = self.state.borrow();
        // The fake holds no tasks: every admitted non-coordinator is unclaimed.
        let members_without_claim = state
            .members
            .iter()
            .filter(|row| matches!(row.status.as_str(), "live" | "reserved"))
            .filter(|row| Some(row.id.as_str()) != coordinator)
            .count();
        Ok(MemberClaimCounts {
            members_without_claim: i64::try_from(members_without_claim).unwrap(),
            members_dead: count(&state.members, |status| status == "dead"),
        })
    }

    fn insert_member(&self, member: &NewMember) -> Result<(), BoardError> {
        self.note(format!("insert_member {}", member.reservation));
        let mut state = self.state.borrow_mut();
        if state.members.iter().any(|row| row.id == member.id) {
            return Err(BoardError::new(
                "coordination store unavailable or contended: UNIQUE constraint failed: members.id",
            ));
        }
        state.members.push(MemberRow {
            id: member.id.clone(),
            reservation: Some(member.reservation.clone()),
            status: member.status.as_str().to_owned(),
            pid: member.pid,
            started: member.started.clone(),
            socket: member.socket.clone(),
            launcher: member.launcher.clone(),
        });
        Ok(())
    }
}

impl BoardEvents for MemoryTransaction<'_> {
    fn event(
        &self,
        actor: &str,
        time: f64,
        action: &str,
        detail: &Value,
    ) -> Result<(), BoardError> {
        self.note(format!("event {action}"));
        self.state.borrow_mut().events.push(RecordedEvent {
            actor: actor.to_owned(),
            time,
            action: action.to_owned(),
            detail: detail.clone(),
        });
        Ok(())
    }

    fn control_generation(&self) -> Result<i64, BoardError> {
        let state = self.state.borrow();
        let latest = state
            .events
            .iter()
            .rposition(|event| matches!(event.action.as_str(), "paused" | "resumed"));
        Ok(latest.map_or(0, |index| i64::try_from(index + 1).unwrap()))
    }
}

fn count(members: &[MemberRow], wanted: impl Fn(&str) -> bool) -> i64 {
    i64::try_from(
        members
            .iter()
            .filter(|row| wanted(row.status.as_str()))
            .count(),
    )
    .unwrap()
}

/// Readings in order, then the last one forever.
pub struct SteppingClock {
    readings: Mutex<VecDeque<f64>>,
    last: Mutex<f64>,
}

impl SteppingClock {
    pub fn new(readings: &[f64]) -> Arc<Self> {
        let first = *readings.first().expect("at least one reading");
        Arc::new(Self {
            readings: Mutex::new(readings.iter().copied().collect()),
            last: Mutex::new(first),
        })
    }

    pub fn fixed(now: f64) -> Arc<Self> {
        Self::new(&[now])
    }
}

impl Clock for SteppingClock {
    fn now_seconds(&self) -> f64 {
        let mut last = self.last.lock().unwrap();
        if let Some(next) = self.readings.lock().unwrap().pop_front() {
            *last = next;
        }
        *last
    }
}

/// `format(n, '032x')` for n = 1, 2, …, each draw journalled.
pub struct CounterIds {
    next: Mutex<u64>,
    journal: Journal,
}

impl CounterIds {
    pub fn journalling(journal: &Journal) -> Arc<Self> {
        Arc::new(Self {
            next: Mutex::new(1),
            journal: journal.clone(),
        })
    }
}

impl IdSource for CounterIds {
    fn hex32(&self) -> String {
        let mut next = self.next.lock().unwrap();
        let id = format!("{:032x}", *next);
        *next += 1;
        self.journal.lock().unwrap().push(format!("draw {id}"));
        id
    }
}

/// Compact JSON: stands in for the board codec, whose exact bytes the
/// infrastructure's own tests pin.
pub struct CompactEncoding;

impl BoardEncoding for CompactEncoding {
    fn encode(&self, value: &Value) -> Result<String, BoardError> {
        Ok(serde_json::to_string(value).unwrap())
    }
}

/// A running run coordinated by `parent` with a live `parent` member.
pub fn running_board(deadline: f64) -> BoardState {
    BoardState {
        run: Some(StoredRun {
            id: "run-1".to_owned(),
            record: RunRecord {
                status: RunState::RUNNING,
                coordinator: "parent".to_owned(),
                deadline,
                member_limit: 2,
                outcome: None,
                outcome_reason: None,
            },
            contract: RunContract {
                goal: "goal".to_owned(),
                constraints: Value::Array(Vec::new()),
                criteria: Value::Array(Vec::new()),
                member_limit: 2,
                deadline,
            },
        }),
        members: vec![member_row("parent", "live")],
        events: Vec::new(),
    }
}

pub fn member_row(id: &str, status: &str) -> MemberRow {
    MemberRow {
        id: id.to_owned(),
        reservation: Some(format!("{id}-reservation")),
        status: status.to_owned(),
        pid: None,
        started: None,
        socket: None,
        launcher: None,
    }
}
