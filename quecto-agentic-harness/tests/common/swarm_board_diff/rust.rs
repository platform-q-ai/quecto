//! The Rust side of the differential harness: the same call interface as
//! `python.rs`, over `swarm_board_dispatch::call` and handles composed by
//! `composition::swarm::build_swarm_board_handles_with` on a clock the step
//! sets and a counter that draws Python's id sequence. The harness never
//! constructs a use case.
use std::path::Path;
use std::sync::{Arc, Mutex};

use quecto::application::swarm::dto::BoardLocation;
use quecto::application::swarm::ports::{Clock, IdSource};
use quecto::composition::swarm::{SwarmBoardHandles, build_swarm_board_handles_with};
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use quecto::infrastructure::tools::swarm_board_dispatch::call;
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

pub struct RustBoard {
    handles: SwarmBoardHandles,
    clock: Arc<StepClock>,
}

impl RustBoard {
    pub fn open(database: &Path, checkout: &Path) -> Self {
        let clock = Arc::new(StepClock::default());
        let location = BoardLocation {
            database: database.to_path_buf(),
            checkout: checkout.to_path_buf(),
        };
        let handles = build_swarm_board_handles_with(
            Arc::new(SqliteBoardRepository::new(&location)),
            clock.clone(),
            Arc::new(CounterIds::default()),
        );
        Self { handles, clock }
    }

    /// One board call as `member` at `now`, its arguments the JSON text
    /// `args`.
    pub fn call_text(&self, member: &str, method: &str, args: &str, now: f64) -> Outcome {
        match serde_json::from_str::<Value>(args) {
            Ok(args) => self.call(member, method, &args, now),
            Err(error) => Outcome::Refused(format!("arguments: {error}")),
        }
    }

    /// One board call as `member` at `now`.
    pub fn call(&self, member: &str, method: &str, args: &Value, now: f64) -> Outcome {
        *self.clock.0.lock().unwrap() = now;
        match call(&self.handles, member, method, args.clone()) {
            Ok(value) => Outcome::Ok(value),
            Err(refusal) => Outcome::Refused(refusal.0),
        }
    }
}
