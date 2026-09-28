//! Steps and the per-step comparison of the two boards.
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::Outcome;
use super::dump::{first_difference, logical_dump};
use super::python::PyBoard;
use super::rust::RustBoard;

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
    }
}

/// Runs a [`sql`] step's statement on `database`.
fn edit(database: &Path, statement: &str) -> Outcome {
    rusqlite::Connection::open(database)
        .and_then(|connection| connection.execute_batch(statement))
        .unwrap_or_else(|error| panic!("{statement} on {}: {error}", database.display()));
    Outcome::Ok(Value::Null)
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

/// Runs `steps` on both boards, calling `after(index, rust_database,
/// rust_outcome)` once each step has run on both and before they are
/// compared (the harness's self-tests tamper with the Rust side's file or
/// answer there); the first difference is the error.
pub fn try_run_both(
    steps: &[Step],
    mut after: impl FnMut(usize, &Path, &mut Outcome),
) -> Result<(), String> {
    let dir = tempfile::tempdir().expect("a directory for the two boards");
    let python_side = Side::new(dir.path(), "python");
    let rust_side = Side::new(dir.path(), "rust");
    let mut python = PyBoard::start(&python_side.database, &python_side.root, dir.path());
    let rust = RustBoard::open(&rust_side.database, &rust_side.root);
    for (index, step) in steps.iter().enumerate() {
        let context = || {
            format!(
                "step {index}: {} as {} with {} at {}",
                step.method, step.member, step.args, step.now
            )
        };
        let (python_outcome, mut rust_outcome) = if step.method == SQL_STEP {
            (
                edit(&python_side.database, &step.args),
                edit(&rust_side.database, &step.args),
            )
        } else {
            (
                python_side.neutral(python.call(&step.member, &step.method, &step.args, step.now)),
                rust_side.neutral(rust.call_text(&step.member, &step.method, &step.args, step.now)),
            )
        };
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
