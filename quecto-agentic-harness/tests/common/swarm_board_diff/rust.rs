//! The Rust side of the differential harness: the same call interface as
//! `python.rs`, over `swarm_board_dispatch::call` and handles composed by
//! `composition::swarm::build_swarm_board_handles_with` on a clock the step
//! sets and a counter that draws Python's id sequence, with the file
//! reservations' paths normalised in the side's own checkout. The harness
//! never constructs a use case. [`RustBoard::open_recorded`] composes the
//! same handles with the event log on (`composition::swarm::with_event_log`,
//! the store's own meter) over an in-memory log, so every scenario also
//! runs with each call measured and recorded (#2303 review M1).
use std::path::Path;
use std::sync::{Arc, Mutex};

use quecto::application::swarm::dto::BoardLocation;
use quecto::application::swarm::ports::{BoardOpLog, Clock, IdSource};
use quecto::composition::swarm::{
    SwarmBoardHandles, board_wire, build_swarm_board_handles_with, with_event_log,
};
use quecto::domain::swarm::BoardOpObservation;
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use quecto::infrastructure::tools::swarm_board_dispatch::call;
use quecto::infrastructure::tools::swarm_board_ops::{member_arguments, wire_text};
use quecto::infrastructure::workspace::checkout_paths::ResolvedCheckout;
use serde_json::Value;

use super::Outcome;

/// The step's instant: every clock read within one call answers it.
#[derive(Default)]
struct StepClock(Mutex<f64>);

impl Clock for StepClock {
    fn now_seconds(&self) -> f64 {
        *self.0.lock().unwrap()
    }
}

/// `format(n, '032x')` for n = 1, 2, …, as the Python driver's `uuid4`.
#[derive(Default)]
struct CounterIds(Mutex<u64>);

impl IdSource for CounterIds {
    fn hex32(&self) -> String {
        let mut drawn = self.0.lock().unwrap();
        *drawn += 1;
        format!("{:032x}", *drawn)
    }
}

/// The event log, in memory: every `swarm_op` recorded, in order.
#[derive(Default)]
pub struct RecordedOps(Mutex<Vec<BoardOpObservation>>);

impl BoardOpLog for RecordedOps {
    fn record(&self, observation: BoardOpObservation) {
        self.0.lock().unwrap().push(observation);
    }

    /// No summary this test checks is written.
    fn summarize(&self, _summary: quecto::domain::swarm::SwarmRunSummary) {}

    fn dropped(&self, _records: u64) {}

    fn take_unnoted(&self) -> u64 {
        0
    }
}

impl RecordedOps {
    /// The records written so far, taken.
    pub fn take(&self) -> Vec<BoardOpObservation> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

fn location(database: &Path, checkout: &Path) -> BoardLocation {
    BoardLocation {
        database: database.to_path_buf(),
        checkout: checkout.to_path_buf(),
    }
}

pub struct RustBoard {
    handles: SwarmBoardHandles,
    clock: Arc<StepClock>,
}

impl RustBoard {
    /// The board with the event log on, recording in `log`.
    pub fn open_recorded(database: &Path, checkout: &Path, log: Arc<RecordedOps>) -> Self {
        let clock = Arc::new(StepClock::default());
        let handles = with_event_log(
            SqliteBoardRepository::new(&location(database, checkout)),
            clock.clone(),
            Arc::new(CounterIds::default()),
            Arc::new(ResolvedCheckout::new(checkout)),
            log,
        );
        Self { handles, clock }
    }

    pub fn open(database: &Path, checkout: &Path) -> Self {
        Self::open_after(database, checkout, 0)
    }

    /// The board whose counter has already drawn `drawn` ids: its next id
    /// is `format(drawn + 1, '032x')`, the one a Python writer that drew
    /// `drawn` would draw next.
    pub fn open_after(database: &Path, checkout: &Path, drawn: u64) -> Self {
        let clock = Arc::new(StepClock::default());
        let handles = build_swarm_board_handles_with(
            Arc::new(SqliteBoardRepository::new(&location(database, checkout))),
            clock.clone(),
            Arc::new(CounterIds(Mutex::new(drawn))),
            Arc::new(ResolvedCheckout::new(checkout)),
        );
        Self { handles, clock }
    }

    /// One board call as `member` at `now`, its arguments the JSON text
    /// `args`, read as the structured `swarm` ops read a member's text
    /// (`swarm_board_ops::member_arguments`, #2279): as Python's
    /// `json.loads` parses it (`py_json::decode`: `-0` is the integer 0,
    /// `1e400` is infinite, a lone surrogate escape is kept), then handed to
    /// the dispatcher, which takes a `serde_json::Value`. A text neither
    /// parser reads, or a value no `Value` holds, is refused there, before
    /// any board call (the `arguments_beyond_a_serde_value` divergence).
    /// Also whether the call reached the dispatcher: a text refused there
    /// never does (the tool records that refusal itself).
    pub fn call_text(&self, member: &str, method: &str, args: &str, now: f64) -> (Outcome, bool) {
        match member_arguments(&board_wire(), args) {
            Ok(args) => (self.call(member, method, &args, now), true),
            Err(refusal) => (Outcome::Refused(refusal), false),
        }
    }

    /// `value` as the structured ops write an answer on the wire
    /// (`swarm_board_ops::wire_text`, #2279).
    pub fn wire_text(&self, value: &Value) -> String {
        wire_text(&board_wire(), value).expect("a board answer is writable")
    }

    /// One board call as `member` at `now`.
    pub fn call(&self, member: &str, method: &str, args: &Value, now: f64) -> Outcome {
        *self.clock.0.lock().unwrap() = now;
        match call(&self.handles, member, method, args.clone()) {
            Ok(value) => Outcome::Ok(value),
            Err(refusal) => Outcome::Refused(refusal.message().to_owned()),
        }
    }
}
