//! Test support (compiled only under `cfg(test)`): in-memory doubles of the
//! board ports for the use-case and gate tests, in the manner of Python's
//! `MemoryRepository` (`tests/swarm_policy_test.py`): a transaction works
//! on a copy of the state that replaces it only when the work succeeds, and
//! every write and id draw is journalled in order.
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::application::swarm::dto::{
    LaunchIdentity, MemberClaimCounts, MemberRow, NewMember, NewRun, RunContract, RunOwnerRow,
    RunStatusRow,
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

    fn run_exists(&self) -> Result<bool, BoardError> {
        Ok(self.state.borrow().run.is_some())
    }

    fn run_owner(&self) -> Result<Option<RunOwnerRow>, BoardError> {
        Ok(self.state.borrow().run.as_ref().map(|run| RunOwnerRow {
            status: run
                .record
                .status
                .as_ref()
                .map(|status| status.as_str().to_owned()),
            coordinator: run.record.coordinator.clone(),
        }))
    }

    fn run_status(&self) -> Result<Option<RunStatusRow>, BoardError> {
        Ok(self.state.borrow().run.as_ref().map(|run| RunStatusRow {
            id: Some(run.id.clone()),
            status: run
                .record
                .status
                .as_ref()
                .map(|status| status.as_str().to_owned()),
            deadline: Value::from(run.record.deadline),
            coordinator: run.record.coordinator.clone(),
            outcome: run.record.outcome.clone(),
        }))
    }

    fn run_coordinator(&self) -> Result<Option<Option<String>>, BoardError> {
        Ok(self
            .state
            .borrow()
            .run
            .as_ref()
            .map(|run| run.record.coordinator.clone()))
    }

    fn insert_run(&self, run: &NewRun) -> Result<(), BoardError> {
        self.note(format!("insert_run {}", run.id));
        let mut state = self.state.borrow_mut();
        assert!(state.run.is_none(), "the board holds one run");
        state.run = Some(StoredRun {
            id: run.id.clone(),
            record: RunRecord {
                status: Some(run.status.clone()),
                coordinator: Some(run.coordinator.clone()),
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
        run.record.status = Some(RunState::RUNNING);
        run.contract = contract.clone();
        Ok(())
    }

    fn propose_outcome(&self, outcome: &str, reason: &str) -> Result<(), BoardError> {
        self.note(format!("propose_outcome {outcome}"));
        let mut state = self.state.borrow_mut();
        let run = state.run.as_mut().expect("a run to pause");
        run.record.status = Some(RunState::PAUSED);
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
            .find(|row| row.text("id") == Some(id))
            .map(|row| MemberRecord {
                id: id.to_owned(),
                status: row.text("status").map(MemberState::new),
                reservation: row.text("reservation").map(str::to_owned),
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
            .filter(|row| matches!(row.text("status"), Some("live" | "reserved")))
            .filter(|row| row.text("id") != coordinator)
            .count();
        Ok(MemberClaimCounts {
            members_without_claim: i64::try_from(members_without_claim).unwrap(),
            members_dead: count(&state.members, |status| status == "dead"),
        })
    }

    fn insert_member(&self, member: &NewMember) -> Result<(), BoardError> {
        self.note(format!("insert_member {}", member.reservation));
        let mut state = self.state.borrow_mut();
        if state
            .members
            .iter()
            .any(|row| row.text("id") == Some(member.id.as_str()))
        {
            return Err(BoardError::new(
                "coordination store unavailable or contended: UNIQUE constraint failed: members.id",
            ));
        }
        state.members.push(stored_member(
            &member.id,
            Value::String(member.reservation.clone()),
            member.status.as_str(),
            [
                integer_affinity(&member.pid),
                text_affinity(&member.started),
                text_affinity(&member.socket),
                member.launcher.clone().map_or(Value::Null, Value::String),
            ],
        ));
        Ok(())
    }

    fn member_row(
        &self,
        id: &Value,
        reservation: Option<&Value>,
    ) -> Result<Option<MemberRow>, BoardError> {
        let id = text_affinity(id);
        let reservation = reservation.map(text_affinity);
        Ok(self
            .state
            .borrow()
            .members
            .iter()
            .find(|row| {
                // SQL `=` never matches a NULL.
                !id.is_null()
                    && row.get("id") == Some(&id)
                    && reservation.as_ref().is_none_or(|wanted| {
                        !wanted.is_null() && row.get("reservation") == Some(wanted)
                    })
            })
            .cloned())
    }

    fn reserve_member(
        &self,
        id: &str,
        reservation: &Value,
        launcher: &str,
    ) -> Result<(), BoardError> {
        self.note(format!("reserve_member {id} by {launcher}"));
        let mut state = self.state.borrow_mut();
        if state.members.iter().any(|row| row.text("id") == Some(id)) {
            return Err(BoardError::new(
                "coordination store unavailable or contended: UNIQUE constraint failed: members.id",
            ));
        }
        state.members.push(stored_member(
            id,
            text_affinity(reservation),
            "reserved",
            [
                Value::Null,
                Value::Null,
                Value::Null,
                Value::String(launcher.to_owned()),
            ],
        ));
        Ok(())
    }

    fn activate_member(
        &self,
        id: &Value,
        launch: &LaunchIdentity,
        socket: &Value,
    ) -> Result<(), BoardError> {
        self.note(format!("activate_member {}", shown(id)));
        self.update(id, |row| {
            set(row, "status", Value::from("live"));
            set(row, "pid", integer_affinity(&launch.pid));
            set(row, "started", text_affinity(&launch.started));
            set(row, "socket", text_affinity(socket));
        });
        Ok(())
    }

    fn record_launch(&self, id: &Value, launch: &LaunchIdentity) -> Result<(), BoardError> {
        self.note(format!("record_launch {}", shown(id)));
        self.update(id, |row| {
            set(row, "pid", integer_affinity(&launch.pid));
            set(row, "started", text_affinity(&launch.started));
        });
        Ok(())
    }

    fn mark_member_dead_unlaunched(&self, id: &Value) -> Result<(), BoardError> {
        self.note(format!("mark_member_dead_unlaunched {}", shown(id)));
        self.update(id, |row| set(row, "status", Value::from("dead")));
        Ok(())
    }

    fn set_socket(&self, id: &str, socket: &Value) -> Result<(), BoardError> {
        self.note(format!("set_socket {id}"));
        self.update(&Value::from(id), |row| {
            set(row, "socket", text_affinity(socket));
        });
        Ok(())
    }
}

/// A member id in a note: its text, or its JSON.
fn shown(id: &Value) -> String {
    match id {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

impl MemoryTransaction<'_> {
    /// `UPDATE members SET … WHERE id=?`: every row with that id.
    fn update(&self, id: &Value, change: impl Fn(&mut MemberRow)) {
        let id = text_affinity(id);
        let mut state = self.state.borrow_mut();
        for row in &mut state.members {
            if !id.is_null() && row.get("id") == Some(&id) {
                change(row);
            }
        }
    }
}

/// Sets `column` of `row`, which the board's rows all hold.
fn set(row: &mut MemberRow, column: &str, value: Value) {
    let slot = row
        .columns
        .iter_mut()
        .find(|(name, _)| name == column)
        .expect("the board's member rows hold every column");
    slot.1 = value;
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

/// Roughly what an INTEGER column keeps of a bound value: a boolean is its
/// integer, anything else as given (the SQLite adapter's contract tests
/// pin the real affinity).
fn integer_affinity(value: &Value) -> Value {
    match value {
        Value::Bool(flag) => Value::from(i64::from(*flag)),
        other => other.clone(),
    }
}

/// Roughly what a TEXT column keeps of a bound value: NULL, or its text.
fn text_affinity(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::String(text) => Value::String(text.clone()),
        Value::Bool(flag) => Value::String(u8::from(*flag).to_string()),
        other => Value::String(other.to_string()),
    }
}

/// A `members` row in the board's column order: `id, reservation, status`,
/// then `pid, started, socket, launcher` as `rest` gives them.
pub fn stored_member(id: &str, reservation: Value, status: &str, rest: [Value; 4]) -> MemberRow {
    let [pid, started, socket, launcher] = rest;
    MemberRow {
        columns: vec![
            ("id".to_owned(), Value::String(id.to_owned())),
            ("reservation".to_owned(), reservation),
            ("status".to_owned(), Value::String(status.to_owned())),
            ("pid".to_owned(), pid),
            ("started".to_owned(), started),
            ("socket".to_owned(), socket),
            ("launcher".to_owned(), launcher),
        ],
    }
}

fn count(members: &[MemberRow], wanted: impl Fn(&str) -> bool) -> i64 {
    i64::try_from(
        members
            .iter()
            .filter(|row| row.text("status").is_some_and(&wanted))
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

/// Stands in for the board codec: sorted keys, compact separators and
/// `ensure_ascii` escaping, as the real encoder writes them, so a size
/// bound counts the bytes the board stores (`é` is six). Floats keep
/// serde's text (`1e16`, where Python writes `1e+16`): the use cases only
/// measure the encoding, and no test here bounds a float-heavy value.
pub struct CompactEncoding;

impl BoardEncoding for CompactEncoding {
    fn encode(&self, value: &Value) -> Result<String, BoardError> {
        let compact = serde_json::to_string(&sorted(value)).unwrap();
        // Outside strings the text is ASCII; inside, each non-ASCII
        // character becomes its UTF-16 escapes (`\u` and four hex digits).
        let mut escaped = String::with_capacity(compact.len());
        for character in compact.chars() {
            if character.is_ascii() {
                escaped.push(character);
            } else {
                let mut units = [0_u16; 2];
                for unit in character.encode_utf16(&mut units) {
                    escaped.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
        Ok(escaped)
    }
}

/// `value` with every object's keys in code-point order.
fn sorted(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        Value::Object(entries) => {
            let mut keys: Vec<&String> = entries.keys().collect();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), sorted(&entries[key])))
                    .collect(),
            )
        }
        other => other.clone(),
    }
}

/// A running run coordinated by `parent` with a live `parent` member.
pub fn running_board(deadline: f64) -> BoardState {
    BoardState {
        run: Some(StoredRun {
            id: "run-1".to_owned(),
            record: RunRecord {
                status: Some(RunState::RUNNING),
                coordinator: Some("parent".to_owned()),
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
    stored_member(
        id,
        Value::String(format!("{id}-reservation")),
        status,
        [Value::Null, Value::Null, Value::Null, Value::Null],
    )
}
