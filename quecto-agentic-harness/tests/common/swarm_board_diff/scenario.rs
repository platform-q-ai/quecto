//! Steps, and the per-step comparison of the Rust board with a scenario's
//! golden fixture (#2283): Python's frozen answers (`golden.rs`).
//!
//! Every scenario runs twice (#2303 review M1): once as the board runs by
//! default, and once with the event log on, each Rust call measured by the
//! store's meter and recorded in memory. Both runs must match the golden
//! step for step, by result, refusal text and board file, and the second
//! must record exactly one `swarm_op` per call, busy for a step run with
//! the lock held.
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use std::panic::Location;

use super::Outcome;
use super::dump::{first_difference, logical_dump};
use super::golden::{
    Answer, FULL_DUMP_LIMIT, GOLDEN_DIR, Golden, canonical, dump_of, file_state, fixture_path,
    scenario_of,
};
use super::python::PyBoard;
use super::rust::{RecordedOps, RustBoard};

/// How long a step waits on a lock another connection holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    /// Held when the call begins and let go 150 ms later: the call waits it
    /// out.
    WaitedOut,
    /// Held until the call returns: the call gives up at the timeout.
    Throughout,
}

/// One board call: `member` calls `method` with `args` (a JSON array binds
/// positionally, an object by name) at clock `now`. `args` is JSON text,
/// which each side parses itself (#2270 round-3 review L1): Python with
/// `json.loads`, Rust with `py_json::decode`, so a text the two parsers
/// read differently is a difference the harness sees.
#[derive(Clone, Debug)]
pub struct Step {
    pub member: String,
    pub method: String,
    pub args: String,
    pub now: f64,
    /// The lock held on each side's board while it runs this step.
    pub hold: Option<Hold>,
}

/// `step`, run on each side while another connection holds the board's
/// write lock (`BEGIN IMMEDIATE`) as `hold` says.
pub fn held(hold: Hold, step: Step) -> Step {
    Step {
        hold: Some(hold),
        ..step
    }
}

/// A step whose arguments are `args` written as JSON.
pub fn step(member: &str, method: &str, args: Value, now: f64) -> Step {
    step_text(member, method, &args.to_string(), now)
}

/// A step whose arguments are the JSON text `args`, exactly as written:
/// for texts a `Value` cannot carry (`-0`, `1e400`, a lone surrogate
/// escape).
pub fn step_text(member: &str, method: &str, args: &str, now: f64) -> Step {
    Step {
        member: member.to_owned(),
        method: method.to_owned(),
        args: args.to_owned(),
        now,
        hold: None,
    }
}

/// The method name of a [`sql`] step: never a board method (the dispatcher
/// and the Python driver both refuse it), so it cannot shadow one.
const SQL_STEP: &str = "<sql>";

/// Not a board call: `statement` runs on both board files alike, as a file
/// edited outside the board would be, and answers `null` on both sides. A
/// later step then shows how each board reads what the edit left.
pub fn sql(statement: &str) -> Step {
    Step {
        member: String::new(),
        method: SQL_STEP.to_owned(),
        args: statement.to_owned(),
        now: 0.0,
        hold: None,
    }
}

/// The method name of a [`mkdir`] or [`symlink`] step: never a board
/// method either.
const FS_STEP: &str = "<fs>";

/// Not a board call: the directory `path` (relative) is made in both
/// sides' checkouts, and the step answers `null` on both.
pub fn mkdir(path: &str) -> Step {
    fs_step(&serde_json::json!(["mkdir", path]))
}

/// Not a board call: `link` (relative) becomes a symlink to the text
/// `target` in both sides' checkouts, and the step answers `null` on both.
pub fn symlink(link: &str, target: &str) -> Step {
    fs_step(&serde_json::json!(["symlink", link, target]))
}

/// Not a board call: `link` (relative) becomes a symlink to the bytes
/// `target`, which need not be UTF-8 (a worker's filesystem can hold any),
/// in both sides' checkouts; the step answers `null` on both.
pub fn symlink_bytes(link: &str, target: &[u8]) -> Step {
    let hex: String = target.iter().map(|byte| format!("{byte:02x}")).collect();
    fs_step(&serde_json::json!(["symlink-bytes", link, hex]))
}

/// Not a board call: both sides' board files are overwritten with bytes
/// that are no SQLite database, and the step answers `null` on both.
pub fn corrupt() -> Step {
    fs_step(&serde_json::json!(["corrupt"]))
}

fn fs_step(args: &Value) -> Step {
    Step {
        member: String::new(),
        method: FS_STEP.to_owned(),
        args: args.to_string(),
        now: 0.0,
        hold: None,
    }
}

/// Runs an [`FS_STEP`]'s change in the checkout `root`.
fn shape(root: &Path, args: &str) -> Outcome {
    let args: Vec<String> = serde_json::from_str(args).expect("a filesystem step's arguments");
    match args.as_slice() {
        [kind] if kind == "corrupt" => std::fs::write(root.join("swarm.sqlite"), b"corrupt")
            .unwrap_or_else(|error| panic!("corrupt the board: {error}")),
        [kind, path] if kind == "mkdir" => std::fs::create_dir_all(root.join(path))
            .unwrap_or_else(|error| panic!("mkdir {path}: {error}")),
        [kind, link, target] if kind == "symlink" => {
            std::os::unix::fs::symlink(target, root.join(link))
                .unwrap_or_else(|error| panic!("symlink {link} -> {target}: {error}"));
        }
        [kind, link, hex] if kind == "symlink-bytes" => {
            use std::os::unix::ffi::OsStrExt;
            let target = (0..hex.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).expect("a hex byte"))
                .collect::<Vec<u8>>();
            std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(&target), root.join(link))
                .unwrap_or_else(|error| panic!("symlink {link} -> {hex}: {error}"));
        }
        other => panic!("an unknown filesystem step: {other:?}"),
    }
    Outcome::Ok(Value::Null)
}

/// Runs a [`sql`] step's statement on `database`.
fn edit(database: &Path, statement: &str) -> Outcome {
    rusqlite::Connection::open(database)
        .and_then(|connection| connection.execute_batch(statement))
        .unwrap_or_else(|error| panic!("{statement} on {}: {error}", database.display()));
    Outcome::Ok(Value::Null)
}

/// Runs `call` while another connection holds `database`'s write lock as
/// `hold` says (or not at all).
fn with_lock<T>(database: &Path, hold: Option<Hold>, call: impl FnOnce() -> T) -> T {
    let Some(hold) = hold else {
        return call();
    };
    let path = database.to_path_buf();
    let (held, taken) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let holder = thread::spawn(move || {
        let connection = rusqlite::Connection::open(path).expect("a holder connection");
        connection
            .execute_batch("BEGIN IMMEDIATE")
            .expect("the holder takes the lock");
        held.send(()).expect("the step waits for the lock");
        let _told = released.recv_timeout(Duration::from_secs(60));
        connection
            .execute_batch("COMMIT")
            .expect("the holder commits");
    });
    taken
        .recv_timeout(Duration::from_secs(60))
        .expect("the lock is held");
    let releaser = match hold {
        Hold::WaitedOut => {
            let release = release.clone();
            Some(thread::spawn(move || {
                thread::sleep(Duration::from_millis(150));
                let _gone = release.send(());
            }))
        }
        Hold::Throughout => None,
    };
    let value = call();
    let _gone = release.send(());
    if let Some(releaser) = releaser {
        releaser.join().expect("the releaser ends");
    }
    holder.join().expect("the holder ends");
    value
}

/// How the Rust board runs: as by default, or with the event log on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Telemetry {
    Off,
    On,
}

/// One side's board file, in its own directory.
struct Side {
    root: PathBuf,
    database: PathBuf,
}

impl Side {
    fn new(parent: &Path, name: &str) -> Self {
        let root = parent.join(name);
        std::fs::create_dir_all(&root).expect("create a board directory");
        let database = root.join("swarm.sqlite");
        Self { root, database }
    }

    /// Refusals name the board's path (a missing store); each side's own
    /// directory reads as `<board>` so the texts compare.
    fn neutral(&self, outcome: Outcome) -> Outcome {
        let root = self.root.to_string_lossy().into_owned();
        match outcome {
            Outcome::Refused(text) => Outcome::Refused(text.replace(&root, "<board>")),
            other => other,
        }
    }
}

/// How a scenario run treats its golden fixture. Replay is the only mode
/// once the Python board is gone; recording and verifying run it live.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Mode {
    /// Compare the Rust board with the committed fixture.
    Replay,
    /// Run the scenario on the live Python board, write its fixture under
    /// this folder, then compare the Rust board with it.
    Record(PathBuf),
    /// Run the scenario on the live Python board and require the committed
    /// fixture to be what it answers, then compare the Rust board with it.
    Verify,
}

/// `QUECTO_SWARM_GOLDEN`: unset for replay, `record` (into
/// `QUECTO_SWARM_GOLDEN_DIR`, or the committed folder) or `verify`.
fn mode() -> Mode {
    match std::env::var("QUECTO_SWARM_GOLDEN").ok().as_deref() {
        None => Mode::Replay,
        Some("record") => Mode::Record(
            std::env::var_os("QUECTO_SWARM_GOLDEN_DIR")
                .map_or_else(|| PathBuf::from(GOLDEN_DIR), PathBuf::from),
        ),
        Some("verify") => Mode::Verify,
        Some(other) => panic!("QUECTO_SWARM_GOLDEN is record or verify, not {other}"),
    }
}

/// The fixture the scenario `steps`, run from `scenario`'s file, is
/// compared with, as the mode says.
fn golden_for(scenario: &str, steps: &[Step]) -> Result<(Golden, PathBuf), String> {
    refuse_held_unlocked_reads(steps)?;
    match mode() {
        Mode::Replay => Ok((
            Golden::load(Path::new(GOLDEN_DIR), scenario, steps)?,
            PathBuf::from(GOLDEN_DIR),
        )),
        Mode::Record(dir) => {
            let golden = record_python(steps);
            golden.save(&dir, scenario, steps);
            Ok((golden, dir))
        }
        Mode::Verify => {
            let stored = Golden::load(Path::new(GOLDEN_DIR), scenario, steps)?;
            let live = record_python(steps);
            match live == stored {
                true => Ok((stored, PathBuf::from(GOLDEN_DIR))),
                false => Err(format!(
                    "the golden {} is not what the live Python board answers:\n  golden {stored:?}\n  live   {live:?}",
                    fixture_path(Path::new(GOLDEN_DIR), scenario, steps).display()
                )),
            }
        }
    }
}

/// Runs `steps` on the Rust board and panics with the first difference
/// from the scenario's golden fixture (Python's frozen answers).
#[track_caller]
pub fn run_golden(steps: &[Step]) {
    let scenario = scenario_of(Location::caller().file());
    if let Err(difference) = compare_golden(&scenario, steps, |_, _, _| {}) {
        panic!("{difference}");
    }
}

/// Runs `steps` (board calls only) on the Rust board and panics at the
/// first step whose answer differs from the golden's as the member reads
/// it (#2279): a result as the Rust board's wire text
/// (`swarm_board_ops::wire_text`, which the structured ops answer with)
/// against the text Python's `json.dumps` wrote, byte for byte, and a
/// refusal by its text. `run_golden` compares the same steps' values and
/// files; this compares their text.
#[track_caller]
pub fn run_golden_wire(steps: &[Step]) {
    let scenario = scenario_of(Location::caller().file());
    let (golden, _) = golden_for(&scenario, steps).unwrap_or_else(|problem| panic!("{problem}"));
    let dir = tempfile::tempdir().expect("a directory for the board");
    let rust_side = Side::new(dir.path(), "rust");
    let rust = RustBoard::open(&rust_side.database, &rust_side.root);
    for (index, (step, answer)) in steps.iter().zip(&golden.answers).enumerate() {
        assert!(
            step.method != SQL_STEP && step.method != FS_STEP && step.hold.is_none(),
            "step {index}: a wire scenario makes board calls only"
        );
        let rust_outcome = rust
            .call_text(&step.member, &step.method, &step.args, step.now)
            .0;
        let context = format!(
            "step {index}: {} as {} with {}",
            step.method, step.member, step.args
        );
        match (answer, rust_outcome) {
            (Answer::Ok(text), Outcome::Ok(value)) => assert_eq!(
                text,
                &rust.wire_text(&value),
                "{context}: the wire texts differ"
            ),
            (answer, rust_outcome) => assert_eq!(
                answer.outcome(),
                rust_side.neutral(rust_outcome),
                "{context}"
            ),
        }
    }
}

/// Runs `steps` on a Rust board alone, each granted but the last: its
/// answer. For a divergence, whose Python side the comparison shows.
pub fn run_rust(steps: &[Step]) -> Outcome {
    let dir = tempfile::tempdir().expect("a directory for the board");
    let side = Side::new(dir.path(), "rust");
    let rust = RustBoard::open(&side.database, &side.root);
    let mut last = Outcome::Ok(Value::Null);
    for step in steps {
        assert!(matches!(last, Outcome::Ok(_)), "before {step:?}: {last:?}");
        last = if step.method == SQL_STEP {
            edit(&side.database, &step.args)
        } else if step.method == FS_STEP {
            shape(&side.root, &step.args)
        } else {
            side.neutral(
                rust.call_text(&step.member, &step.method, &step.args, step.now)
                    .0,
            )
        };
    }
    last
}

/// What the Python board answered to the last of `steps`, each before it
/// granted (#2340: a divergence's pin asserts Python's answer rather than
/// naming it), from the scenario's golden fixture (#2283; recorded on the
/// live Python board, as every fixture was).
#[track_caller]
pub fn golden_answer(steps: &[Step]) -> Outcome {
    let scenario = scenario_of(Location::caller().file());
    let (golden, _) = golden_for(&scenario, steps).unwrap_or_else(|problem| panic!("{problem}"));
    let (last, before) = golden.answers.split_last().expect("a scenario has a step");
    for (step, answer) in steps.iter().zip(before) {
        let outcome = answer.outcome();
        assert!(
            matches!(outcome, Outcome::Ok(_)),
            "before {step:?}: {outcome:?}"
        );
    }
    last.outcome()
}

/// A held step of an [`UNLOCKED_READS`] method is no differential scenario
/// (#2338): refused before any fixture is read (or recorded).
fn refuse_held_unlocked_reads(steps: &[Step]) -> Result<(), String> {
    match steps
        .iter()
        .enumerate()
        .find(|(_, step)| step.hold.is_some() && UNLOCKED_READS.contains(&step.method.as_str()))
    {
        Some((index, step)) => Err(format!(
            "step {index}: {} as {} with {} at {}: a held step of a method read without the \
             write lock is no differential scenario: Python refuses it while the lock is \
             held, Rust answers it unbusied (ADR-0030, #2338; pinned by \
             `a_snapshot_does_not_wait_for_a_writers_lock`)",
            step.method, step.member, step.args, step.now
        )),
        None => Ok(()),
    }
}

/// Runs `steps` on a Rust board alone (each held as it says) and answers
/// every step, refused or not: what the Rust board answered at each step
/// of a scenario `run_golden` has compared (so Python's answers too).
pub fn rust_answers(steps: &[Step]) -> Vec<Outcome> {
    let dir = tempfile::tempdir().expect("a directory for the board");
    let side = Side::new(dir.path(), "rust");
    let rust = RustBoard::open(&side.database, &side.root);
    steps
        .iter()
        .map(|step| {
            if step.method == SQL_STEP {
                edit(&side.database, &step.args)
            } else if step.method == FS_STEP {
                shape(&side.root, &step.args)
            } else {
                let answer = with_lock(&side.database, step.hold, || {
                    rust.call_text(&step.member, &step.method, &step.args, step.now)
                        .0
                });
                side.neutral(answer)
            }
        })
        .collect()
}

/// Runs `steps` on the Rust board, calling `after(index, rust_database,
/// rust_outcome)` once each step has run and before it is compared with
/// the golden (the harness's self-tests tamper with the Rust side's file
/// or answer there); the first difference is the error. The steps run
/// twice, with the event log off and then on (see the module docs).
#[track_caller]
pub fn try_run_golden(
    steps: &[Step],
    after: impl FnMut(usize, &Path, &mut Outcome),
) -> Result<(), String> {
    let scenario = scenario_of(Location::caller().file());
    compare_golden(&scenario, steps, after)
}

/// [`run_golden`] against the fixture `scenario` holds under `dir` rather
/// than the committed one, as it is on disk (never recorded), with no
/// hook: the first difference is the error.
pub fn try_run_golden_in(dir: &Path, scenario: &str, steps: &[Step]) -> Result<(), String> {
    let golden = Golden::load(dir, scenario, steps)?;
    replay(steps, &golden, |_, _, _| {})
}

fn compare_golden(
    scenario: &str,
    steps: &[Step],
    after: impl FnMut(usize, &Path, &mut Outcome),
) -> Result<(), String> {
    let (golden, _) = golden_for(scenario, steps)?;
    replay(steps, &golden, after)
}

fn replay(
    steps: &[Step],
    golden: &Golden,
    mut after: impl FnMut(usize, &Path, &mut Outcome),
) -> Result<(), String> {
    run_in(steps, golden, Telemetry::Off, &mut after)?;
    run_in(steps, golden, Telemetry::On, &mut after)
        .map_err(|difference| format!("with the event log on, {difference}"))
}

fn run_in(
    steps: &[Step],
    golden: &Golden,
    telemetry: Telemetry,
    after: &mut impl FnMut(usize, &Path, &mut Outcome),
) -> Result<(), String> {
    assert_eq!(
        golden.answers.len(),
        steps.len(),
        "a golden answer per step"
    );
    let dir = tempfile::tempdir().expect("a directory for the board");
    let rust_side = Side::new(dir.path(), "rust");
    let log = Arc::new(RecordedOps::default());
    let rust = match telemetry {
        Telemetry::Off => RustBoard::open(&rust_side.database, &rust_side.root),
        Telemetry::On => {
            RustBoard::open_recorded(&rust_side.database, &rust_side.root, log.clone())
        }
    };
    let last = steps.len().saturating_sub(1);
    for (index, step) in steps.iter().enumerate() {
        let context = || {
            format!(
                "step {index}: {} as {} with {} at {}",
                step.method, step.member, step.args, step.now
            )
        };
        if step.hold.is_some() && UNLOCKED_READS.contains(&step.method.as_str()) {
            return Err(format!(
                "{}: a held step of a method read without the write lock is no \
                 differential scenario: Python refuses it while the lock is held, \
                 Rust answers it unbusied (ADR-0030, #2338; pinned by \
                 `a_snapshot_does_not_wait_for_a_writers_lock`)",
                context()
            ));
        }
        let (mut rust_outcome, dispatched) = if step.method == SQL_STEP {
            (edit(&rust_side.database, &step.args), false)
        } else if step.method == FS_STEP {
            (shape(&rust_side.root, &step.args), false)
        } else {
            let (rust_outcome, dispatched) = with_lock(&rust_side.database, step.hold, || {
                rust.call_text(&step.member, &step.method, &step.args, step.now)
            });
            (rust_side.neutral(rust_outcome), dispatched)
        };
        if telemetry == Telemetry::On {
            recorded(&log, dispatched, step.hold)
                .map_err(|problem| format!("{}: {problem}", context()))?;
        }
        after(index, &rust_side.database, &mut rust_outcome);
        let expected = golden.answers[index].outcome();
        if let Outcome::Raised(raised) = &expected {
            return Err(format!(
                "{}: Python raised {raised}\n  rust   {rust_outcome:?}",
                context()
            ));
        }
        if expected != rust_outcome {
            return Err(format!(
                "{}: results differ\n  golden {expected:?}\n  rust   {rust_outcome:?}",
                context()
            ));
        }
        let state = file_state(&rust_side.database);
        match (&golden.files[index], &state) {
            (expected, state) if expected == state => {}
            (Some(expected), Some(state))
                if expected.starts_with("dump:") && state.starts_with("dump:") =>
            {
                let detail = match (index == last, &golden.dump) {
                    (true, Some(dump)) => {
                        first_difference(&dump_of(dump), &logical_dump(&rust_side.database))
                            .unwrap_or_else(|| "the dumps' digests differ".to_owned())
                    }
                    _ => "the logical dump differs from the golden's".to_owned(),
                };
                return Err(format!("{}: boards differ: {detail}", context()));
            }
            (Some(_), Some(_)) => {
                return Err(format!("{}: board files differ as bytes", context()));
            }
            (expected, state) => {
                return Err(format!(
                    "{}: board file exists in golden: {}, in rust: {}",
                    context(),
                    expected.is_some(),
                    state.is_some()
                ));
            }
        }
    }
    Ok(())
}

/// The methods the Rust board reads in a read transaction that takes no
/// write lock whenever the gate has nothing to write (#2338, ADR-0030):
/// a writer holding the lock neither refuses nor slows them, where
/// Python's `BEGIN IMMEDIATE` waits and then refuses `database is locked`.
/// A held step of one is refused by the harness, never compared.
pub const UNLOCKED_READS: [&str; 3] = ["_snapshot", "_watch", "_event_cursor"];

/// Runs `steps` on a fresh board through `call` (each step held as it
/// says) and records the fixture: each answer, the board file after each
/// step, and the final dump. `call` answers a board call and, for a
/// result, its wire text.
fn record(
    steps: &[Step],
    side: &Side,
    mut call: impl FnMut(&Step) -> (Outcome, Option<String>),
) -> Golden {
    let mut golden = Golden {
        answers: Vec::with_capacity(steps.len()),
        files: Vec::with_capacity(steps.len()),
        dump: None,
    };
    for step in steps {
        let answer = if step.method == SQL_STEP {
            edit(&side.database, &step.args);
            Answer::Ok("null".to_owned())
        } else if step.method == FS_STEP {
            shape(&side.root, &step.args);
            Answer::Ok("null".to_owned())
        } else {
            let (outcome, text) = with_lock(&side.database, step.hold, || call(step));
            match (side.neutral(outcome), text) {
                (Outcome::Ok(_), Some(text)) => Answer::Ok(text),
                (Outcome::Ok(value), None) => {
                    panic!("{step:?}: a result without its text: {value}")
                }
                (Outcome::Refused(text), _) => Answer::Refused(text),
                (Outcome::Raised(text), _) => Answer::Raised(text),
            }
        };
        golden.answers.push(answer);
        golden.files.push(file_state(&side.database));
    }
    golden.dump = match golden.files.last() {
        Some(Some(state)) if state.starts_with("dump:") => {
            Some(canonical(&logical_dump(&side.database))).filter(|dump| {
                serde_json::to_string(dump).map_or(0, |text| text.len()) <= FULL_DUMP_LIMIT
            })
        }
        _ => None,
    };
    golden
}

/// The fixture the live Python board records for `steps`.
fn record_python(steps: &[Step]) -> Golden {
    let dir = tempfile::tempdir().expect("a directory for the board");
    let side = Side::new(dir.path(), "python");
    let mut python = PyBoard::start(&side.database, &side.root, dir.path());
    record(steps, &side, |step| {
        python.call_wire(&step.member, &step.method, &step.args, step.now)
    })
}

/// The fixture the Rust board would record for `steps`: for the harness's
/// own tests, which need a fixture whose every value is known to match.
pub fn record_rust(steps: &[Step]) -> Golden {
    let dir = tempfile::tempdir().expect("a directory for the board");
    let side = Side::new(dir.path(), "rust");
    let rust = RustBoard::open(&side.database, &side.root);
    record(steps, &side, |step| {
        let (outcome, _) = rust.call_text(&step.member, &step.method, &step.args, step.now);
        let text = match &outcome {
            Outcome::Ok(value) => Some(rust.wire_text(value)),
            Outcome::Refused(_) | Outcome::Raised(_) => None,
        };
        (outcome, text)
    })
}

/// The event log holds exactly one record for a dispatched call (none for
/// a step that never reached the dispatcher), busy when the lock was held.
fn recorded(log: &RecordedOps, dispatched: bool, hold: Option<Hold>) -> Result<(), String> {
    let records = log.take();
    let expected = usize::from(dispatched);
    if records.len() != expected {
        return Err(format!(
            "{} swarm_op records, {expected} expected: {records:?}",
            records.len()
        ));
    }
    match (hold, records.first()) {
        (None, _) | (Some(_), None) => Ok(()),
        (Some(_), Some(record)) => match record.busy {
            Some(true) => Ok(()),
            Some(false) | None => Err(format!(
                "the lock was held, the record is not busy: {record:?}"
            )),
        },
    }
}
