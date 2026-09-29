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
//! and never sequences two use cases. It serves `_status` and `_snapshot`
//! (#2270) and the harness's membership methods `_admit`, `_activate`,
//! `_record_launch`, `_release_unlaunched` and `_socket` (#2271); later
//! slices add methods, and S13 wires it into `SwarmContext`. Every
//! membership argument (member, reservation, pid, start time, socket)
//! reaches the use case as the JSON value passed (#2271 round-1 review
//! M1): Python binds each untyped, so the store binds it as Python's
//! `sqlite3` does and the board compares it by Python's `==`; no type is
//! refused here. `create_run`, `bootstrap_run` and `bootstrap_join` (the
//! transactional halves of `create` and `_bootstrap`, and `_bootstrap`'s
//! join, which the differential harness drives) exist only in `test` and
//! `test-support` builds: a production build parses none of them and
//! refuses each as an unknown method. The task and claim methods
//! `task_create`, `dependencies`, `claim` and `release` (#2272) are served
//! by [`tasks`], with the test-only `task_raw` (`Tasks._task` inside a
//! read-only operation, before S12 adds owner liveness to `task`).
//!
//! What S12 must keep when it adds the summaries Python answers with:
//!
//! - `create` commits and then calls `summary()`, which can still raise: a
//!   created run answered with a refusal.
//! - `_bootstrap`'s join ends every branch with `coordinator.summary()`,
//!   after its writes commit, so an admitted or re-activated member can be
//!   answered with a refusal too.
//! - The join's already-live branch writes nothing and still ends with
//!   `coordinator.summary()`, whose gate can refuse (a coordinator that is
//!   nobody, or dead). `Joined::AlreadyLive` carries the coordinator the
//!   join read, so S12 runs that gate as it without reading the run again.
//!
//! `JoinRun` composes `AdmitMember` and `ActivateMember` by a deliberate
//! decision (#2271 round-1 review L3): Python's `join_process` calls those
//! two `Workbench` methods, so the use case sequences them, and the
//! dispatcher still calls one use case per method.
//!
//! Each call leaves one `tracing` record on [`TELEMETRY_TARGET`] (DEBUG for
//! a read-only method, INFO otherwise) with the
//! method, the member id, the outcome, the decision taken and the
//! duration: ids, kinds and durations only, never argument text. The
//! member id is the board identity the harness assigned the caller (the
//! `members.id` every board row names), not a secret; it is still passed
//! through [`Redacted`] so an id shaped like a credential is masked in the
//! log, as every telemetry field that carries caller-chosen text is.
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::application::audit::ports::AuditSink;
use crate::application::swarm::dto::{
    ActivateMemberRequest, AdmissionDecision, AdmitMemberRequest, LaunchIdentity, MemberRow,
    RecordMemberLaunchRequest, RegisterMemberSocketRequest, ReleaseUnlaunchedMemberRequest,
    RunSnapshotView, RunStatusView,
};
#[cfg(any(test, feature = "test-support"))]
use crate::application::swarm::dto::{
    BootstrapRunRequest, CreateBranch, CreateRunRequest, JoinRunRequest, Joined,
};
use crate::application::swarm::use_cases::{
    ActivateMember, AdmitMember, BootstrapRun, ClaimTask, CreateRun, CreateTask, JoinRun,
    ReadRunSnapshot, ReadRunStatus, ReadTask, RecordMemberLaunch, RegisterMemberSocket,
    ReleaseTask, ReleaseUnlaunchedMember, SetTaskDependencies,
};
use crate::domain::redaction::Redacted;
use crate::domain::swarm::BoardError;

/// The `tracing` target of every board call record.
pub const TELEMETRY_TARGET: &str = "quecto::swarm_board";

/// One handle per board use case, composed once per board file. The
/// `create_run` and `bootstrap_run` handles are served only in test builds
/// (see the module docs); later slices' methods reach them otherwise.
#[derive(Clone)]
pub struct SwarmBoardHandles {
    pub create_run: Arc<CreateRun>,
    pub bootstrap_run: Arc<BootstrapRun>,
    pub read_run_status: Arc<ReadRunStatus>,
    pub read_run_snapshot: Arc<ReadRunSnapshot>,
    pub admit_member: Arc<AdmitMember>,
    pub activate_member: Arc<ActivateMember>,
    pub record_member_launch: Arc<RecordMemberLaunch>,
    pub release_unlaunched_member: Arc<ReleaseUnlaunchedMember>,
    pub register_member_socket: Arc<RegisterMemberSocket>,
    /// Served only in test builds as `bootstrap_join` until S12 adds the
    /// summary `_bootstrap` answers with.
    pub join_run: Arc<JoinRun>,
    pub create_task: Arc<CreateTask>,
    pub set_task_dependencies: Arc<SetTaskDependencies>,
    pub claim_task: Arc<ClaimTask>,
    pub release_task: Arc<ReleaseTask>,
    /// Served only in test builds as `task_raw` until S12 adds the owner
    /// liveness `task` answers with.
    pub read_task: Arc<ReadTask>,
    /// The event log every call records a `swarm_op` in (#2303), only
    /// when it is switched on (`telemetry.event_log.enabled`).
    pub event_log: Option<Arc<dyn AuditSink>>,
}

impl std::fmt::Debug for SwarmBoardHandles {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SwarmBoardHandles")
            .finish_non_exhaustive()
    }
}

/// Every board method this dispatcher serves, by name (#2303).
pub const BOARD_OPS: &[&str] = &[
    "_status",
    "_snapshot",
    #[cfg(any(test, feature = "test-support"))]
    "create_run",
    #[cfg(any(test, feature = "test-support"))]
    "bootstrap_run",
];

/// The board methods this dispatcher serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Method {
    Status,
    Snapshot,
    Admit,
    Activate,
    RecordLaunch,
    ReleaseUnlaunched,
    Socket,
    TaskCreate,
    Dependencies,
    Claim,
    Release,
    #[cfg(any(test, feature = "test-support"))]
    CreateRun,
    #[cfg(any(test, feature = "test-support"))]
    BootstrapRun,
    #[cfg(any(test, feature = "test-support"))]
    BootstrapJoin,
    #[cfg(any(test, feature = "test-support"))]
    TaskRaw,
}

/// The telemetry level of a call (#2270 round-3 review N3): a method that
/// only reads the board records at DEBUG; anything else (a mutation, or a
/// name that is no method) at INFO.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
    Read,
    Mutation,
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

const ADMIT: [Parameter; 2] = [required("member"), required("reservation")];
const ACTIVATE: [Parameter; 5] = [
    required("member"),
    required("reservation"),
    required("pid"),
    required("started"),
    required("socket"),
];
const RECORD_LAUNCH: [Parameter; 4] = [
    required("member"),
    required("reservation"),
    required("pid"),
    required("started"),
];
const RELEASE_UNLAUNCHED: [Parameter; 1] = [required("member")];
const SOCKET: [Parameter; 1] = [required("socket")];

impl Method {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "_status" => Some(Self::Status),
            "_snapshot" => Some(Self::Snapshot),
            "_admit" => Some(Self::Admit),
            "_activate" => Some(Self::Activate),
            "_record_launch" => Some(Self::RecordLaunch),
            "_release_unlaunched" => Some(Self::ReleaseUnlaunched),
            "_socket" => Some(Self::Socket),
            "task_create" => Some(Self::TaskCreate),
            "dependencies" => Some(Self::Dependencies),
            "claim" => Some(Self::Claim),
            "release" => Some(Self::Release),
            #[cfg(any(test, feature = "test-support"))]
            "create_run" => Some(Self::CreateRun),
            #[cfg(any(test, feature = "test-support"))]
            "bootstrap_run" => Some(Self::BootstrapRun),
            #[cfg(any(test, feature = "test-support"))]
            "bootstrap_join" => Some(Self::BootstrapJoin),
            #[cfg(any(test, feature = "test-support"))]
            "task_raw" => Some(Self::TaskRaw),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Status => "_status",
            Self::Snapshot => "_snapshot",
            Self::Admit => "_admit",
            Self::Activate => "_activate",
            Self::RecordLaunch => "_record_launch",
            Self::ReleaseUnlaunched => "_release_unlaunched",
            Self::Socket => "_socket",
            Self::TaskCreate => "task_create",
            Self::Dependencies => "dependencies",
            Self::Claim => "claim",
            Self::Release => "release",
            #[cfg(any(test, feature = "test-support"))]
            Self::CreateRun => "create_run",
            #[cfg(any(test, feature = "test-support"))]
            Self::BootstrapRun => "bootstrap_run",
            #[cfg(any(test, feature = "test-support"))]
            Self::BootstrapJoin => "bootstrap_join",
            #[cfg(any(test, feature = "test-support"))]
            Self::TaskRaw => "task_raw",
        }
    }

    /// [`Level::Read`] only for the methods listed as read-only.
    fn level(self) -> Level {
        match self {
            Self::Status | Self::Snapshot => Level::Read,
            Self::Admit
            | Self::Activate
            | Self::RecordLaunch
            | Self::ReleaseUnlaunched
            | Self::Socket
            | Self::TaskCreate
            | Self::Dependencies
            | Self::Claim
            | Self::Release => Level::Mutation,
            #[cfg(any(test, feature = "test-support"))]
            Self::CreateRun | Self::BootstrapRun | Self::BootstrapJoin => Level::Mutation,
            #[cfg(any(test, feature = "test-support"))]
            Self::TaskRaw => Level::Read,
        }
    }

    /// The Python signature, `self` left out.
    fn parameters(self) -> &'static [Parameter] {
        match self {
            Self::Status | Self::Snapshot => &[],
            Self::Admit => &ADMIT,
            Self::Activate => &ACTIVATE,
            Self::RecordLaunch => &RECORD_LAUNCH,
            Self::ReleaseUnlaunched => &RELEASE_UNLAUNCHED,
            Self::Socket => &SOCKET,
            Self::TaskCreate => &tasks::TASK_CREATE,
            Self::Dependencies => &tasks::DEPENDENCIES,
            Self::Claim => &tasks::CLAIM,
            Self::Release => &tasks::RELEASE,
            #[cfg(any(test, feature = "test-support"))]
            Self::CreateRun => &test_only::CREATE,
            #[cfg(any(test, feature = "test-support"))]
            Self::BootstrapRun => &test_only::BOOTSTRAP,
            #[cfg(any(test, feature = "test-support"))]
            Self::BootstrapJoin => &test_only::JOIN,
            #[cfg(any(test, feature = "test-support"))]
            Self::TaskRaw => &tasks::CLAIM,
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
/// `args` must be the member's JSON text as `py_json::decode` reads it
/// (`PyJson::to_value`), never `serde_json::from_str`: serde's default
/// float parser is not correctly rounded (`9007199254740993.0` parses as
/// `9007199254740994.0`, where Python's `json.loads` gives
/// `9007199254740992.0`), so the board's `python_equal` would compare a
/// value Python never held and answer a retry or a refusal differently
/// (S13/S14; pinned by `serde_float_parsing_would_change_python_equal`).
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
        record(
            "unknown",
            Level::Mutation,
            member,
            "refused",
            "none",
            started.elapsed(),
        );
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
                known.level(),
                member,
                "ok",
                served.decision,
                started.elapsed(),
            );
            Ok(served.value)
        }
        Err(refusal) => {
            record(
                known.name(),
                known.level(),
                member,
                "refused",
                "none",
                started.elapsed(),
            );
            Err(refusal)
        }
    }
}

fn record(op: &str, level: Level, member: &str, outcome: &str, decision: &str, elapsed: Duration) {
    let duration_us = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
    let member = Redacted::from(member);
    let member = member.as_str();
    match level {
        Level::Read => tracing::debug!(
            target: TELEMETRY_TARGET,
            op,
            member,
            outcome,
            decision,
            duration_us,
            "swarm board call"
        ),
        Level::Mutation => tracing::info!(
            target: TELEMETRY_TARGET,
            op,
            member,
            outcome,
            decision,
            duration_us,
            "swarm board call"
        ),
    }
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
            value: status(handles.read_run_status.execute()?),
            decision: "read",
        }),
        Method::Snapshot => Ok(Served {
            value: snapshot(handles.read_run_snapshot.execute(member)?)?,
            decision: "read",
        }),
        Method::Admit => admit(handles, member, arguments),
        Method::Activate => activate(handles, member, arguments),
        Method::RecordLaunch => record_launch(handles, member, arguments),
        Method::ReleaseUnlaunched => release_unlaunched(handles, member, arguments),
        Method::Socket => socket(handles, member, arguments),
        Method::TaskCreate => tasks::task_create(handles, member, arguments),
        Method::Dependencies => tasks::dependencies(handles, member, arguments),
        Method::Claim => tasks::claim(handles, member, arguments),
        Method::Release => tasks::release(handles, member, arguments),
        #[cfg(any(test, feature = "test-support"))]
        Method::CreateRun => test_only::create_run(handles, member, arguments),
        #[cfg(any(test, feature = "test-support"))]
        Method::BootstrapRun => test_only::bootstrap_run(handles, member, arguments),
        #[cfg(any(test, feature = "test-support"))]
        Method::BootstrapJoin => test_only::bootstrap_join(handles, member, arguments),
        #[cfg(any(test, feature = "test-support"))]
        Method::TaskRaw => tasks::task_raw(handles, member, arguments),
    }
}

/// `_admit(member, reservation)`: the member's row, as `dict(row)`. The
/// member stays the JSON value passed, which the board bounds.
fn admit(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [member, reservation] = take(arguments)?;
    let admitted = handles.admit_member.execute(AdmitMemberRequest {
        actor: actor.to_owned(),
        member,
        reservation,
    })?;
    Ok(Served {
        value: member_row(admitted.row),
        decision: match admitted.decision {
            AdmissionDecision::Reserved => "reserved",
            AdmissionDecision::Retry => "retry",
        },
    })
}

fn activate(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [member, reservation, pid, started, socket] = take(arguments)?;
    handles.activate_member.execute(ActivateMemberRequest {
        actor: actor.to_owned(),
        member,
        reservation,
        launch: LaunchIdentity { pid, started },
        socket,
    })?;
    Ok(done("activated"))
}

fn record_launch(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [member, reservation, pid, started] = take(arguments)?;
    handles
        .record_member_launch
        .execute(RecordMemberLaunchRequest {
            actor: actor.to_owned(),
            member,
            reservation,
            launch: LaunchIdentity { pid, started },
        })?;
    Ok(done("recorded"))
}

fn release_unlaunched(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [member] = take(arguments)?;
    handles
        .release_unlaunched_member
        .execute(ReleaseUnlaunchedMemberRequest {
            actor: actor.to_owned(),
            member,
        })?;
    Ok(done("released"))
}

fn socket(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [socket] = take(arguments)?;
    handles
        .register_member_socket
        .execute(RegisterMemberSocketRequest {
            actor: actor.to_owned(),
            socket,
        })?;
    Ok(done("registered"))
}

/// A method that returns `None`, and the decision it took.
fn done(decision: &'static str) -> Served {
    Served {
        value: Value::Null,
        decision,
    }
}

/// The bound arguments as an array of the signature's length.
fn take<const N: usize>(arguments: Vec<Value>) -> Result<[Value; N], BoardError> {
    arguments
        .try_into()
        .map_err(|_| BoardError::new("swarm board bound the wrong number of arguments"))
}

/// The test-only methods: their signatures and their serving.
#[cfg(any(test, feature = "test-support"))]
mod test_only {
    use serde_json::Value;

    use super::{
        BootstrapRunRequest, CreateBranch, CreateRunRequest, JoinRunRequest, Joined,
        LaunchIdentity, Parameter, Served, SwarmBoardHandles, done, required, take,
    };
    use crate::domain::swarm::BoardError;

    pub(super) const CREATE: [Parameter; 5] = [
        required("goal"),
        required("constraints"),
        required("criteria"),
        required("member_limit"),
        required("deadline"),
    ];
    pub(super) const BOOTSTRAP: [Parameter; 3] =
        [required("pid"), required("started"), required("socket")];
    /// `_bootstrap(pid, started, socket, reservation=None)`'s signature.
    pub(super) const JOIN: [Parameter; 4] = [
        required("pid"),
        required("started"),
        required("socket"),
        Parameter {
            name: "reservation",
            default: Some(|| Value::Null),
        },
    ];

    pub(super) fn create_run(
        handles: &SwarmBoardHandles,
        member: &str,
        arguments: Vec<Value>,
    ) -> Result<Served, BoardError> {
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

    /// The three values reach the store as the member passed them: Python
    /// binds them untyped (epic P3), so a type is never refused here.
    pub(super) fn bootstrap_run(
        handles: &SwarmBoardHandles,
        member: &str,
        arguments: Vec<Value>,
    ) -> Result<Served, BoardError> {
        let [pid, started, socket] = take(arguments)?;
        let bootstrapped = handles.bootstrap_run.execute(BootstrapRunRequest {
            member: member.to_owned(),
            pid,
            started,
            socket,
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

    /// `join_process` for the calling member: `null`, as the driver alias
    /// answers until S12 adds the summary.
    pub(super) fn bootstrap_join(
        handles: &SwarmBoardHandles,
        member: &str,
        arguments: Vec<Value>,
    ) -> Result<Served, BoardError> {
        let [pid, started, socket, reservation] = take(arguments)?;
        let joined = handles.join_run.execute(JoinRunRequest {
            member: member.to_owned(),
            reservation,
            launch: LaunchIdentity { pid, started },
            socket,
        })?;
        Ok(done(match joined {
            Joined::Admitted => "admitted",
            Joined::AlreadyLive { .. } => "already_live",
            Joined::Reactivated => "reactivated",
        }))
    }
}

/// `_status`'s dict: the counts, then the run's fields as stored.
fn status(view: RunStatusView) -> Value {
    let text = |value: Option<String>| value.map_or(Value::Null, Value::String);
    object([
        (
            "members_without_claim",
            Value::from(view.counts.members_without_claim),
        ),
        ("members_dead", Value::from(view.counts.members_dead)),
        ("id", text(view.id)),
        ("status", text(view.status)),
        ("deadline", view.deadline),
        ("coordinator", text(view.coordinator)),
        ("outcome", text(view.outcome)),
    ])
}

/// `_snapshot`'s dict, with each member row as `dict(row)`.
fn snapshot(view: RunSnapshotView) -> Result<Value, BoardError> {
    Ok(object([
        ("status", view.status.map_or(Value::Null, Value::String)),
        (
            "coordinator",
            view.coordinator.map_or(Value::Null, Value::String),
        ),
        ("outcome", view.outcome.map_or(Value::Null, Value::String)),
        ("control_generation", Value::from(view.control_generation)),
        ("deadline", float(view.deadline)?),
        (
            "members",
            Value::Array(view.members.into_iter().map(member_row).collect()),
        ),
    ]))
}

/// `dict(row)`: every column, in table order.
fn member_row(row: MemberRow) -> Value {
    Value::Object(row.columns.into_iter().collect())
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

#[path = "swarm_board_dispatch_tasks.rs"]
mod tasks;

#[cfg(test)]
#[path = "swarm_board_dispatch_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "swarm_board_dispatch_telemetry_tests.rs"]
mod telemetry_tests;
