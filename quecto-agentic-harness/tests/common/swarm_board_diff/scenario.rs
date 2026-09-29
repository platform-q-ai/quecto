//! Steps and the per-step comparison of the two boards.
//!
//! Every scenario runs twice (#2303 review M1): once as the board runs by
//! default, and once with the event log on, each Rust call measured by the
//! store's meter and recorded in memory. Both runs must match Python step
//! for step, by result, refusal text and dump, and the second must record
//! exactly one `swarm_op` per call, busy for a step run with the lock held.
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use serde_json::Value;

use super::Outcome;
use super::dump::{first_difference, logical_dump};
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

fn fs_step(args: &Value) -> Step {
    Step {
        member: String::new(),
        method: FS_STEP.to_owned(),
        args: args.to_string(),
        now: 0.0,
    }
}

/// Runs an [`FS_STEP`]'s change in the checkout `root`.
fn shape(root: &Path, args: &str) -> Outcome {
    let args: Vec<String> = serde_json::from_str(args).expect("a filesystem step's arguments");
    match args.as_slice() {
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

/// Runs `steps` on both boards and panics with the first difference.
pub fn run_both(steps: &[Step]) {
    if let Err(difference) = try_run_both(steps, |_, _, _| {}) {
        panic!("{difference}");
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

/// Runs `steps` on both boards, calling `after(index, rust_database,
/// rust_outcome)` once each step has run on both and before they are
/// compared (the harness's self-tests tamper with the Rust side's file or
/// answer there); the first difference is the error. The steps run twice,
/// with the event log off and then on (see the module docs).
pub fn try_run_both(
    steps: &[Step],
    mut after: impl FnMut(usize, &Path, &mut Outcome),
) -> Result<(), String> {
    run_in(steps, Telemetry::Off, &mut after)?;
    run_in(steps, Telemetry::On, &mut after)
        .map_err(|difference| format!("with the event log on, {difference}"))
}

fn run_in(
    steps: &[Step],
    telemetry: Telemetry,
    after: &mut impl FnMut(usize, &Path, &mut Outcome),
) -> Result<(), String> {
    let dir = tempfile::tempdir().expect("a directory for the two boards");
    let python_side = Side::new(dir.path(), "python");
    let rust_side = Side::new(dir.path(), "rust");
    let mut python = PyBoard::start(&python_side.database, &python_side.root, dir.path());
    let log = Arc::new(RecordedOps::default());
    let rust = match telemetry {
        Telemetry::Off => RustBoard::open(&rust_side.database, &rust_side.root),
        Telemetry::On => {
            RustBoard::open_recorded(&rust_side.database, &rust_side.root, log.clone())
        }
    };
    for (index, step) in steps.iter().enumerate() {
        let context = || {
            format!(
                "step {index}: {} as {} with {} at {}",
                step.method, step.member, step.args, step.now
            )
        };
        let (python_outcome, mut rust_outcome, dispatched) = if step.method == SQL_STEP {
            (
                edit(&python_side.database, &step.args),
                edit(&rust_side.database, &step.args),
                false,
            )
        } else if step.method == FS_STEP {
            (
                shape(&python_side.root, &step.args),
                shape(&rust_side.root, &step.args),
            )
        } else {
            let python_outcome = with_lock(&python_side.database, step.hold, || {
                python.call(&step.member, &step.method, &step.args, step.now)
            });
            let (rust_outcome, dispatched) = with_lock(&rust_side.database, step.hold, || {
                rust.call_text(&step.member, &step.method, &step.args, step.now)
            });
            (
                python_side.neutral(python_outcome),
                rust_side.neutral(rust_outcome),
                dispatched,
            )
        };
        if telemetry == Telemetry::On {
            recorded(&log, dispatched, step.hold)
                .map_err(|problem| format!("{}: {problem}", context()))?;
        }
        after(index, &rust_side.database, &mut rust_outcome);
        if let Outcome::Raised(raised) = &python_outcome {
            return Err(format!("{}: Python raised {raised}", context()));
        }
        if python_outcome != rust_outcome {
            return Err(format!(
                "{}: results differ\n  python {python_outcome:?}\n  rust   {rust_outcome:?}",
                context()
            ));
        }
        match (python_side.database.exists(), rust_side.database.exists()) {
            (true, true) => {
                let difference = first_difference(
                    &logical_dump(&python_side.database),
                    &logical_dump(&rust_side.database),
                );
                if let Some(difference) = difference {
                    return Err(format!("{}: boards differ: {difference}", context()));
                }
            }
            (false, false) => {}
            (python_exists, rust_exists) => {
                return Err(format!(
                    "{}: board file exists in python: {python_exists}, in rust: {rust_exists}",
                    context()
                ));
            }
        }
    }
    Ok(())
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
