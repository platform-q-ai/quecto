//! The board's method dispatch (#2270, epic #2265): the tool-side inbound
//! adapter that turns one board call — a member, a Python method name and
//! its JSON arguments — into one use-case invocation, and renders the result
//! as the JSON value the Python method returned (key order included).
//!
//! Arguments bind as Python binds them to the method's signature: a JSON
//! array positionally, a JSON object by name, defaults filling the rest.
//! Binding errors are calling syntax (epic P2) and carry this module's own
//! text; every refusal the board raises keeps its exact text.
//!
//! The dispatcher holds composed handles only ([`SwarmBoardHandles`], built
//! by `composition::swarm`): it never constructs a use case or an adapter,
//! and never sequences two use cases. This slice serves `_status`,
//! `_snapshot` and the test-only `create_run` and `bootstrap_run` (the
//! transactional halves of `create` and `_bootstrap`); later slices add
//! methods, and S13 wires it into `SwarmContext`.
//!
//! Each call leaves one `tracing` record on [`TELEMETRY_TARGET`] with the
//! method, the (redacted) member id, the outcome, the decision taken and
//! the duration: ids, kinds and durations only, never argument text.
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::application::swarm::dto::{
    BootstrapRunRequest, CreateBranch, CreateRunRequest, MemberRow, RunSnapshotView, RunStatusView,
    StatusDeadline,
};
use crate::application::swarm::use_cases::{
    BootstrapRun, CreateRun, ReadRunSnapshot, ReadRunStatus,
};
use crate::domain::redaction::Redacted;
use crate::domain::swarm::BoardError;

/// The `tracing` target of every board call record.
pub const TELEMETRY_TARGET: &str = "quecto::swarm_board";

/// One handle per board use case, composed once per board file.
#[derive(Clone)]
pub struct SwarmBoardHandles {
    pub create_run: Arc<CreateRun>,
    pub bootstrap_run: Arc<BootstrapRun>,
    pub read_run_status: Arc<ReadRunStatus>,
    pub read_run_snapshot: Arc<ReadRunSnapshot>,
}

impl std::fmt::Debug for SwarmBoardHandles {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SwarmBoardHandles")
            .finish_non_exhaustive()
    }
}

/// The board methods this dispatcher serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    Status,
    Snapshot,
    CreateRun,
    BootstrapRun,
}

/// One parameter of a Python signature: required, or with a default.
#[derive(Clone, Copy)]
struct Parameter {
    name: &'static str,
    default: Option<fn() -> Value>,
}

const fn required(name: &'static str) -> Parameter {
    Parameter {
        name,
        default: None,
    }
}

impl Method {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "_status" => Some(Self::Status),
            "_snapshot" => Some(Self::Snapshot),
            "create_run" => Some(Self::CreateRun),
            "bootstrap_run" => Some(Self::BootstrapRun),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Status => "_status",
            Self::Snapshot => "_snapshot",
            Self::CreateRun => "create_run",
            Self::BootstrapRun => "bootstrap_run",
        }
    }

    /// The Python signature, `self` left out.
    fn parameters(self) -> &'static [Parameter] {
        const CREATE: [Parameter; 5] = [
            required("goal"),
            required("constraints"),
            required("criteria"),
            required("member_limit"),
            required("deadline"),
        ];
        const BOOTSTRAP: [Parameter; 3] =
            [required("pid"), required("started"), required("socket")];
        match self {
            Self::Status | Self::Snapshot => &[],
            Self::CreateRun => &CREATE,
            Self::BootstrapRun => &BOOTSTRAP,
        }
    }
}

/// What a served call decided, for its telemetry record.
struct Served {
    value: Value,
    decision: &'static str,
}

/// Invokes `method` as `member` with `args`: one use case, its result in
/// Python's JSON shape.
///
/// # Errors
/// An unknown method or an argument that does not bind (this module's
/// text), or the board's refusal (Python's text).
pub fn call(
    handles: &SwarmBoardHandles,
    member: &str,
    method: &str,
    args: Value,
) -> Result<Value, BoardError> {
    let started = Instant::now();
    let Some(known) = Method::parse(method) else {
        record("unknown", member, "refused", "none", started.elapsed());
        return Err(BoardError::new(format!(
            "swarm board has no method {method}"
        )));
    };
    let served = bind(known, known.parameters(), args)
        .and_then(|arguments| serve(handles, member, known, arguments));
    match served {
        Ok(served) => {
            record(
                known.name(),
                member,
                "ok",
                served.decision,
                started.elapsed(),
            );
            Ok(served.value)
        }
        Err(refusal) => {
            record(known.name(), member, "refused", "none", started.elapsed());
            Err(refusal)
        }
    }
}

fn record(op: &str, member: &str, outcome: &str, decision: &str, elapsed: Duration) {
    let duration_us = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
    let member = Redacted::from(member);
    tracing::info!(
        target: TELEMETRY_TARGET,
        op,
        member = member.as_str(),
        outcome,
        decision,
        duration_us,
        "swarm board call"
    );
}

/// Binds `args` to `parameters` as Python binds a call: positionally from
/// an array, by name from an object, then each unbound parameter's default.
fn bind(method: Method, parameters: &[Parameter], args: Value) -> Result<Vec<Value>, BoardError> {
    let name = method.name();
    let mut slots: Vec<Option<Value>> = vec![None; parameters.len()];
    match args {
        Value::Array(values) => {
            if values.len() > parameters.len() {
                return Err(BoardError::new(format!(
                    "{name}: takes {} arguments, {} given",
                    parameters.len(),
                    values.len()
                )));
            }
            for (slot, value) in slots.iter_mut().zip(values) {
                *slot = Some(value);
            }
        }
        Value::Object(fields) => {
            for (key, value) in fields {
                let Some(index) = parameters.iter().position(|p| p.name == key) else {
                    return Err(BoardError::new(format!(
                        "{name}: unexpected argument {key}"
                    )));
                };
                slots[index] = Some(value);
            }
        }
        _ => {
            return Err(BoardError::new(format!(
                "{name}: arguments must be a JSON array or object"
            )));
        }
    }
    slots
        .into_iter()
        .zip(parameters)
        .map(|(slot, parameter)| match (slot, parameter.default) {
            (Some(value), _) => Ok(value),
            (None, Some(default)) => Ok(default()),
            (None, None) => Err(BoardError::new(format!(
                "{name}: missing required argument {}",
                parameter.name
            ))),
        })
        .collect()
}

fn serve(
    handles: &SwarmBoardHandles,
    member: &str,
    method: Method,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    debug_assert_eq!(
        arguments.len(),
        method.parameters().len(),
        "every parameter is bound"
    );
    match method {
        Method::Status => Ok(Served {
            value: status(handles.read_run_status.execute()?)?,
            decision: "read",
        }),
        Method::Snapshot => Ok(Served {
            value: snapshot(handles.read_run_snapshot.execute(member)?)?,
            decision: "read",
        }),
        Method::CreateRun => {
            let [goal, constraints, criteria, member_limit, deadline] = take(arguments)?;
            let created = handles.create_run.execute(CreateRunRequest {
                member: member.to_owned(),
                goal,
                constraints,
                criteria,
                member_limit,
                deadline,
            })?;
            Ok(Served {
                value: Value::Null,
                decision: match created.branch {
                    CreateBranch::Fresh => "fresh",
                    CreateBranch::OverSetup => "over_setup",
                },
            })
        }
        Method::BootstrapRun => {
            let [pid, started, socket] = take(arguments)?;
            let bootstrapped = handles.bootstrap_run.execute(BootstrapRunRequest {
                member: member.to_owned(),
                pid: optional_integer(method, "pid", pid)?,
                started: optional_text(method, "started", started)?,
                socket: optional_text(method, "socket", socket)?,
            })?;
            Ok(Served {
                value: Value::Null,
                decision: if bootstrapped.created {
                    "created"
                } else {
                    "existing"
                },
            })
        }
    }
}

/// The bound arguments as an array of the signature's length.
fn take<const N: usize>(arguments: Vec<Value>) -> Result<[Value; N], BoardError> {
    arguments
        .try_into()
        .map_err(|_| BoardError::new("swarm board bound the wrong number of arguments"))
}

fn optional_integer(method: Method, name: &str, value: Value) -> Result<Option<i64>, BoardError> {
    match value {
        Value::Null => Ok(None),
        Value::Number(number) if number.is_i64() => Ok(number.as_i64()),
        _ => Err(BoardError::new(format!(
            "{}: {name} must be an integer or null",
            method.name()
        ))),
    }
}

fn optional_text(method: Method, name: &str, value: Value) -> Result<Option<String>, BoardError> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text)),
        _ => Err(BoardError::new(format!(
            "{}: {name} must be a string or null",
            method.name()
        ))),
    }
}

/// `_status`'s dict: the counts, then the run's fields.
fn status(view: RunStatusView) -> Result<Value, BoardError> {
    let deadline = match view.deadline {
        StatusDeadline::NoRun => Value::from(0),
        StatusDeadline::Stored(deadline) => float(deadline)?,
    };
    Ok(object([
        (
            "members_without_claim",
            Value::from(view.counts.members_without_claim),
        ),
        ("members_dead", Value::from(view.counts.members_dead)),
        ("id", view.id.map_or(Value::Null, Value::String)),
        ("status", Value::String(view.status)),
        ("deadline", deadline),
        (
            "coordinator",
            view.coordinator.map_or(Value::Null, Value::String),
        ),
        ("outcome", view.outcome.map_or(Value::Null, Value::String)),
    ]))
}

/// `_snapshot`'s dict, with each member row as `dict(row)`.
fn snapshot(view: RunSnapshotView) -> Result<Value, BoardError> {
    Ok(object([
        ("status", Value::String(view.status)),
        ("coordinator", Value::String(view.coordinator)),
        ("outcome", view.outcome.map_or(Value::Null, Value::String)),
        ("control_generation", Value::from(view.control_generation)),
        ("deadline", float(view.deadline)?),
        (
            "members",
            Value::Array(view.members.into_iter().map(member_row).collect()),
        ),
    ]))
}

fn member_row(row: MemberRow) -> Value {
    let text = |value: Option<String>| value.map_or(Value::Null, Value::String);
    object([
        ("id", Value::String(row.id)),
        ("reservation", text(row.reservation)),
        ("status", Value::String(row.status)),
        ("pid", row.pid.map_or(Value::Null, Value::from)),
        ("started", text(row.started)),
        ("socket", text(row.socket)),
        ("launcher", text(row.launcher)),
    ])
}

fn object<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<Map<String, Value>>(),
    )
}

/// A stored REAL as Python's `json` writes a float. SQLite can hold an
/// infinity that JSON cannot; that is refused rather than written as null.
fn float(value: f64) -> Result<Value, BoardError> {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| BoardError::new(format!("the board holds a non-finite number: {value}")))
}

#[cfg(test)]
#[path = "swarm_board_dispatch_tests.rs"]
mod tests;
