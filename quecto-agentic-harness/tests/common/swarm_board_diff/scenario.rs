//! Steps and the per-step comparison of the two boards.
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::Outcome;
use super::dump::{first_difference, logical_dump};
use super::python::PyBoard;
use super::rust::RustBoard;

/// One board call: `member` calls `method` with `args` (a JSON array binds
/// positionally, an object by name) at clock `now`.
#[derive(Clone, Debug)]
pub struct Step {
    pub member: String,
    pub method: String,
    pub args: Value,
    pub now: f64,
}

pub fn step(member: &str, method: &str, args: Value, now: f64) -> Step {
    Step {
        member: member.to_owned(),
        method: method.to_owned(),
        args,
        now,
    }
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
    if let Err(difference) = try_run_both(steps, |_, _| {}) {
        panic!("{difference}");
    }
}

/// Runs `steps` on both boards, calling `after(index, rust_database)` once
/// each step has run on both and before they are compared (the harness's
/// self-test tampers there); the first difference is the error.
pub fn try_run_both(steps: &[Step], mut after: impl FnMut(usize, &Path)) -> Result<(), String> {
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
        let python_outcome =
            python_side.neutral(python.call(&step.member, &step.method, &step.args, step.now));
        let rust_outcome =
            rust_side.neutral(rust.call(&step.member, &step.method, &step.args, step.now));
        after(index, &rust_side.database);
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
