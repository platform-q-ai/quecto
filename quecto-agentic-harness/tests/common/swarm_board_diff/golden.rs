//! The differential suite's golden fixtures (#2283): what the Python board
//! answered, frozen before its sources were deleted.
//!
//! Each scenario the suite ran on both boards is one file,
//! `tests/fixtures/swarm_board/golden/<test file>/<key>.json`, where the
//! key is the first 16 hex digits of the SHA-256 of the scenario's steps
//! (member, method, argument text, clock, lock hold), so a scenario whose
//! steps change no longer finds its fixture and fails, rather than being
//! compared with another scenario's answers. A fixture holds:
//!
//! - `steps`: the steps, as keyed (checked on load against the caller's);
//! - `answers`: per step, `{"ok": text}` (the result as Python's
//!   `json.dumps` wrote it, which the wire comparison needs byte for
//!   byte), `{"refused": text}` (a `SwarmError`, the board's directory
//!   written `<board>`) or `{"raised": text}` (any other exception);
//! - `files`: per step, the board file after it: `null` (no file),
//!   `"dump:<sha256>"` (the SHA-256 of the logical dump's canonical text)
//!   or `"bytes:<sha256>"` (a file that is no database);
//! - `dump`: the logical dump after the last step, in full (`null` when
//!   the last step left no database, or when the dump's text is longer
//!   than [`FULL_DUMP_LIMIT`], as a 10,000-request ledger's is: its digest
//!   in `files` still guards it), so a difference there is shown row by
//!   row. Earlier steps keep only the digest (the issue's fixture-size
//!   trap): the per-step dump comparison was the point while Python
//!   existed; the final state plus every result is the guard afterwards.
use std::path::{Path, PathBuf};

use rusqlite::types::Value as SqlValue;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::Outcome;
use super::dump::{Dump, logical_dump};
use super::scenario::{Hold, Step};

/// The committed fixtures.
pub const GOLDEN_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/swarm_board/golden"
);

/// The longest final dump (as compact JSON text) a fixture holds in full.
pub const FULL_DUMP_LIMIT: usize = 32 * 1024;

/// One step's recorded answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The result, as Python's `json.dumps` wrote it.
    Ok(String),
    /// A `SwarmError`'s text.
    Refused(String),
    /// Any other exception, as `<type>: <text>`.
    Raised(String),
}

impl Answer {
    /// The answer as the harness compares it: a result read as Python's
    /// `json.loads` reads it (`py_json::decode`, correctly rounded).
    pub fn outcome(&self) -> Outcome {
        match self {
            Self::Ok(text) => Outcome::Ok(decoded(text)),
            Self::Refused(text) => Outcome::Refused(text.clone()),
            Self::Raised(text) => Outcome::Raised(text.clone()),
        }
    }

    fn to_json(&self) -> Value {
        match self {
            Self::Ok(text) => json!({"ok": text}),
            Self::Refused(text) => json!({"refused": text}),
            Self::Raised(text) => json!({"raised": text}),
        }
    }

    fn from_json(value: &Value) -> Result<Self, String> {
        let object = value
            .as_object()
            .filter(|object| object.len() == 1)
            .ok_or_else(|| format!("an answer is one keyed text: {value}"))?;
        let (key, text) = object.iter().next().expect("one entry");
        let text = text
            .as_str()
            .ok_or_else(|| format!("an answer's text is a string: {value}"))?
            .to_owned();
        match key.as_str() {
            "ok" => Ok(Self::Ok(text)),
            "refused" => Ok(Self::Refused(text)),
            "raised" => Ok(Self::Raised(text)),
            other => Err(format!("an unknown answer kind {other}")),
        }
    }
}

/// A result text read as Python's `json.loads` reads it.
pub fn decoded(text: &str) -> Value {
    quecto::infrastructure::persistence::swarm_board::py_json::decode(text)
        .and_then(|value| value.to_value())
        .unwrap_or_else(|error| panic!("a golden result {text:?}: {error}"))
}

/// One scenario's frozen answers.
#[derive(Clone, Debug, PartialEq)]
pub struct Golden {
    pub answers: Vec<Answer>,
    /// The board file after each step (see the module docs).
    pub files: Vec<Option<String>>,
    /// The logical dump after the last step, canonical.
    pub dump: Option<Value>,
}

/// Where one scenario's fixture is: `<dir>/<scenario>/<key>.json`.
pub fn fixture_path(dir: &Path, scenario: &str, steps: &[Step]) -> PathBuf {
    dir.join(scenario).join(format!("{}.json", key(steps)))
}

/// The scenario name of a caller's file: its stem.
pub fn scenario_of(file: &str) -> String {
    Path::new(file)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_else(|| panic!("a caller file with a stem: {file}"))
        .to_owned()
}

/// The steps as a fixture holds them: a clock as Rust's shortest
/// round-tripping digits (a string, so no JSON reader rounds it).
pub fn steps_json(steps: &[Step]) -> Value {
    Value::Array(
        steps
            .iter()
            .map(|step| {
                let hold = match step.hold {
                    None => Value::Null,
                    Some(Hold::WaitedOut) => json!("waited_out"),
                    Some(Hold::Throughout) => json!("throughout"),
                };
                json!([
                    step.member,
                    step.method,
                    step.args,
                    format!("{:?}", step.now),
                    hold
                ])
            })
            .collect(),
    )
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The scenario's key: its steps' digest.
pub fn key(steps: &[Step]) -> String {
    let text = serde_json::to_string(&steps_json(steps)).expect("steps serialize");
    sha256(text.as_bytes())[..16].to_owned()
}

/// Whether `path` holds a SQLite database (or is empty, which SQLite
/// opens as one).
pub fn is_database(path: &Path) -> bool {
    std::fs::read(path)
        .map(|bytes| bytes.is_empty() || bytes.starts_with(b"SQLite format 3\0"))
        .unwrap_or(false)
}

/// The board file at `database` as a fixture records it after a step.
pub fn file_state(database: &Path) -> Option<String> {
    match (database.exists(), is_database(database)) {
        (false, _) => None,
        (true, true) => Some(format!("dump:{}", dump_digest(&logical_dump(database)))),
        (true, false) => {
            let bytes = std::fs::read(database)
                .unwrap_or_else(|error| panic!("read {}: {error}", database.display()));
            Some(format!("bytes:{}", sha256(&bytes)))
        }
    }
}

/// The digest of a dump's canonical text.
pub fn dump_digest(dump: &Dump) -> String {
    let text = serde_json::to_string(&canonical(dump)).expect("a dump serializes");
    sha256(text.as_bytes())
}

/// A dump as JSON, losslessly: each cell `[storage class, value]`, the
/// value tagged by its kind, a REAL as Rust's shortest round-tripping
/// digits (`-0.0` kept), a BLOB (or text that is not UTF-8) as hex.
pub fn canonical(dump: &Dump) -> Value {
    Value::Array(
        dump.iter()
            .map(|(table, rows)| {
                let rows: Vec<Value> = rows
                    .iter()
                    .map(|row| {
                        Value::Array(
                            row.iter()
                                .map(|(class, value)| json!([class, cell(value)]))
                                .collect(),
                        )
                    })
                    .collect();
                json!([table, rows])
            })
            .collect(),
    )
}

fn cell(value: &SqlValue) -> Value {
    match value {
        SqlValue::Null => Value::Null,
        SqlValue::Integer(integer) => json!({"integer": integer}),
        SqlValue::Real(real) => json!({"real": format!("{real:?}")}),
        SqlValue::Text(text) => json!({"text": text}),
        SqlValue::Blob(bytes) => json!({"blob": hex(bytes)}),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).expect("a hex byte"))
        .collect()
}

/// [`canonical`]'s inverse.
pub fn dump_of(canonical: &Value) -> Dump {
    let tables = canonical.as_array().expect("a dump is an array of tables");
    tables
        .iter()
        .map(|table| {
            let name = table[0].as_str().expect("a table name").to_owned();
            let rows = table[1]
                .as_array()
                .expect("a table's rows")
                .iter()
                .map(|row| {
                    row.as_array()
                        .expect("a row's cells")
                        .iter()
                        .map(|pair| {
                            let class = pair[0].as_str().expect("a storage class").to_owned();
                            (class, value_of(&pair[1]))
                        })
                        .collect()
                })
                .collect();
            (name, rows)
        })
        .collect()
}

fn value_of(cell: &Value) -> SqlValue {
    let Some(object) = cell.as_object() else {
        assert!(cell.is_null(), "a cell is null or tagged: {cell}");
        return SqlValue::Null;
    };
    match (
        object.get("integer"),
        object.get("real"),
        object.get("text"),
        object.get("blob"),
    ) {
        (Some(integer), None, None, None) => {
            SqlValue::Integer(integer.as_i64().expect("an integer cell"))
        }
        (None, Some(Value::String(real)), None, None) => {
            SqlValue::Real(real.parse().expect("a real cell's digits"))
        }
        (None, None, Some(Value::String(text)), None) => SqlValue::Text(text.clone()),
        (None, None, None, Some(Value::String(blob))) => SqlValue::Blob(unhex(blob)),
        _ => panic!("an unknown cell {cell}"),
    }
}

impl Golden {
    /// The fixture's text: JSON with one step, answer, file state or dump
    /// row per line, so a fixture reads (and diffs) line by line.
    pub fn to_text(&self, steps: &[Step]) -> String {
        assert_eq!(self.answers.len(), steps.len(), "an answer per step");
        assert_eq!(self.files.len(), steps.len(), "a file state per step");
        let line = |value: &Value| serde_json::to_string(value).expect("a golden serializes");
        let list = |values: Vec<String>, indent: &str| match values.is_empty() {
            true => "[]".to_owned(),
            false => format!(
                "[\n{indent}{}\n{}]",
                values.join(&format!(",\n{indent}")),
                &indent[1..]
            ),
        };
        let steps = match steps_json(steps) {
            Value::Array(steps) => steps.iter().map(line).collect(),
            _ => unreachable!("steps are an array"),
        };
        let answers = self
            .answers
            .iter()
            .map(|answer| line(&answer.to_json()))
            .collect();
        let files = self.files.iter().map(|file| line(&json!(file))).collect();
        let dump = match &self.dump {
            None => "null".to_owned(),
            Some(Value::Array(tables)) => list(
                tables
                    .iter()
                    .map(|table| {
                        let rows = table[1].as_array().expect("a table's rows");
                        format!(
                            "[{}, {}]",
                            line(&table[0]),
                            list(rows.iter().map(line).collect(), "   ")
                        )
                    })
                    .collect(),
                "  ",
            ),
            Some(other) => panic!("a dump is an array of tables: {other}"),
        };
        format!(
            "{{\n\"steps\": {},\n\"answers\": {},\n\"files\": {},\n\"dump\": {}\n}}\n",
            list(steps, " "),
            list(answers, " "),
            list(files, " "),
            dump
        )
    }

    /// The fixture `text` holds for `steps`: its steps must be those.
    pub fn from_text(text: &str, steps: &[Step]) -> Result<Self, String> {
        let value: Value =
            serde_json::from_str(text).map_err(|error| format!("a golden is JSON: {error}"))?;
        if value["steps"] != steps_json(steps) {
            return Err("the golden holds other steps than the scenario's".to_owned());
        }
        let answers = value["answers"]
            .as_array()
            .ok_or("a golden's answers are an array")?
            .iter()
            .map(Answer::from_json)
            .collect::<Result<Vec<_>, _>>()?;
        let files = value["files"]
            .as_array()
            .ok_or("a golden's files are an array")?
            .iter()
            .map(|file| match file {
                Value::Null => Ok(None),
                Value::String(state) => Ok(Some(state.clone())),
                other => Err(format!("a file state is a string or null: {other}")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if answers.len() != steps.len() || files.len() != steps.len() {
            return Err(format!(
                "a golden answers {} steps with {} file states, the scenario has {}",
                answers.len(),
                files.len(),
                steps.len()
            ));
        }
        let dump = match &value["dump"] {
            Value::Null => None,
            dump => Some(dump.clone()),
        };
        Ok(Self {
            answers,
            files,
            dump,
        })
    }

    /// The fixture for `steps` under `dir`.
    pub fn load(dir: &Path, scenario: &str, steps: &[Step]) -> Result<Self, String> {
        let path = fixture_path(dir, scenario, steps);
        let text = std::fs::read_to_string(&path).map_err(|error| {
            format!(
                "no golden fixture {} for this scenario ({error}): the Python board that \
                 recorded the goldens is gone (#2283), so a new or changed differential \
                 scenario has no expected answers; assert the Rust board's answers directly",
                path.display()
            )
        })?;
        Self::from_text(&text, steps).map_err(|problem| format!("{}: {problem}", path.display()))
    }

    /// Writes the fixture for `steps` under `dir`, atomically (a scenario
    /// run twice writes the same text).
    pub fn save(&self, dir: &Path, scenario: &str, steps: &[Step]) {
        let path = fixture_path(dir, scenario, steps);
        let parent = path.parent().expect("a fixture has a folder");
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("create {}: {error}", parent.display()));
        let staged = tempfile::NamedTempFile::new_in(parent).expect("stage a fixture");
        std::fs::write(staged.path(), self.to_text(steps)).expect("write a fixture");
        staged
            .persist(&path)
            .unwrap_or_else(|error| panic!("persist {}: {error}", path.display()));
    }
}
