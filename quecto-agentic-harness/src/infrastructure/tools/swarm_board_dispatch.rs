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
//! and never sequences two use cases. While the event log is on, each call
//! is served by the composed use case [`OverRepository::over`] the call's
//! own metered repository: the graph is composed once, and only the
//! repository is the call's. It serves `_status` and `_snapshot`
//! (#2270) and the harness's membership methods `_admit`, `_activate`,
//! `_record_launch`, `_release_unlaunched` and `_socket` (#2271); later
//! slices add methods, and `SwarmContext` and `HostedStore` call it
//! through `swarm_bridge::SwarmBoard` (#2278). Every
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
//! by [`tasks`], with `task` and the test-only `task_raw` (`task` without
//! the owner liveness #2277 adds); `block`,
//! `unblock`, `submit` and `verify_task` by [`submissions`]; the run
//! control methods `pause`, `resume`, `stop`, `_resume_external`,
//! `_close`, `_extend_deadline`, `_control_status` and `usage_report`
//! (#2273) by [`control`]; `complete`, `revalidate_task`, `amend` and the
//! criterion `evidence` success needs (#2273) by [`completion`]; the
//! usage methods `usage_budget`, `_record_request` and
//! `_request_admission` (#2274) by [`usage`]; and the file reservations
//! `reserve`, `release_files` and `file_owners`, with the coordinator's
//! `recover` and `revoke` (#2275), by [`reservations`]; and the durable
//! messages `send`, `withdraw`, `inbox` and `ack` (#2276) by [`messages`],
//! the wake notifications `_notifications` and `_accept_wake` by [`wakes`],
//! the loss and death records `_quarantine`, `_confirmed_dead` and
//! `_lose_coordinator` (#2277) by [`loss`], and the read models `summary`,
//! `events` and `tasks`, with the summaries `create`, `_join` and
//! `_bootstrap` answer with (#2277), by [`reads`].
//!
//! The summaries keep Python's order: `create` commits and then reads the
//! creator's summary, which can still refuse (a created run answered with
//! a refusal); `_join` and `_bootstrap` end every branch of the join with
//! the coordinator's summary after its writes commit, the already-live
//! branch (which writes nothing) included, so its gate can refuse a
//! coordinator that is nobody.
//!
//! `JoinRun` runs `AdmitMember`'s and `ActivateMember`'s work by a
//! deliberate decision (#2271 round-1 review L3): Python's `join_process`
//! calls those two `Workbench` methods, so the use case sequences them (as
//! shared functions over its own repository, so a metered call measures
//! all of it, #2303), and the dispatcher still calls one use case per
//! method.
//!
//! Each call leaves one `tracing` record on [`TELEMETRY_TARGET`] (DEBUG
//! for a read-only method, unless the token budget warned or paused the
//! run in it; INFO otherwise; WARN for a contended refusal or,
//! while the event log is on, a busy wait over 250 ms) and, when the event
//! log is switched on ([`SwarmBoardHandles::telemetry`], #2303), one
//! `swarm_op` record in it, with the lock wait, busy wait and busy flag
//! the store measured for this call only: the call's own
//! [`MeteredCall`], never state shared with another call. The dispatcher
//! reaches both through application ports ([`BoardCallMeter`],
//! [`BoardOpLog`]) that composition injects: it names no adapter. Both records carry the
//! method, the member id, the outcome (a refusal's [`RefusalKind`]), the
//! decision taken and the duration: ids, kinds, durations and sizes only,
//! never argument text (`swarm_board_telemetry`). The member id is the
//! board identity the caller chose for itself (the `members.id` every
//! board row names, bounded by the board), not a secret; it is still
//! passed through `Redacted` so an id shaped like a credential is masked,
//! as every telemetry field that carries caller-chosen text is, once per
//! member ([`ActorRefs`]: only a caller the board accepted as a member,
//! per [`Method::answers_members_only`], is kept). A method a later slice
//! adds is recorded with no further code: it gets an arm in
//! [`Method::role`] and [`Method::answers_members_only`] and an entry in
//! [`BOARD_OPS`], which the one-record-per-op test walks.
use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;

use crate::application::swarm::ports::{BoardCallMeter, BoardOpLog, BoardRepository};
use crate::application::swarm::use_cases::{
    AcceptWake, AcknowledgeMessage, ActivateMember, AdmitMember, AmendRunContract, BlockTask,
    BootstrapMember, BootstrapRun, ClaimNotifications, ClaimTask, CloseRun, CompleteRun,
    ConfigureUsageBudget, ConfirmMemberDead, CreateRun, CreateTask, ExtendRunDeadline, JoinMember,
    JoinRun, ListFileOwners, ListTasks, LoseCoordinator, OverRepository, PauseRun,
    QuarantineMember, ReadControlStatus, ReadEventCursor, ReadInbox, ReadRequestAdmission,
    ReadRunEvents, ReadRunSnapshot, ReadRunStatus, ReadRunSummary, ReadTask, ReadUsageReport,
    RecordEvidence, RecordMemberLaunch, RecordRequestUsage, RecoverTask, RegisterMemberSocket,
    ReleaseFiles, ReleaseTask, ReleaseUnlaunchedMember, ReserveFiles, ResumeRun,
    ResumeRunExternally, RevalidateTask, RevokeTask, SendMessage, SetTaskDependencies, StopRun,
    SubmitTask, UnblockTask, VerifyTask, WithdrawMessage,
};
use crate::domain::swarm::{BoardError, BoardOpDetail, RefusalKind};

use self::method::{Method, Parameter, required};
pub use super::swarm_board_telemetry::{ActorRefs, TELEMETRY_TARGET};
use super::swarm_board_telemetry::{Caller, Finished, Level, Served, split_committed};

/// One handle per board use case, composed once per board file. The
/// `create_run` and `bootstrap_run` handles are served only in test builds
/// (see the module docs); later slices' methods reach them otherwise.
#[derive(Clone)]
pub struct SwarmBoardHandles {
    pub create_run: Arc<CreateRun>,
    pub bootstrap_run: Arc<BootstrapRun>,
    pub read_run_status: Arc<ReadRunStatus>,
    /// `_event_cursor` (#2279): Rust-only, the structured ops' before and
    /// after read.
    pub read_event_cursor: Arc<ReadEventCursor>,
    pub read_run_snapshot: Arc<ReadRunSnapshot>,
    pub admit_member: Arc<AdmitMember>,
    pub activate_member: Arc<ActivateMember>,
    pub record_member_launch: Arc<RecordMemberLaunch>,
    pub release_unlaunched_member: Arc<ReleaseUnlaunchedMember>,
    pub register_member_socket: Arc<RegisterMemberSocket>,
    /// Served only in test builds, as `bootstrap_join`: `_join` without
    /// its closing summary.
    pub join_run: Arc<JoinRun>,
    pub create_task: Arc<CreateTask>,
    pub set_task_dependencies: Arc<SetTaskDependencies>,
    pub claim_task: Arc<ClaimTask>,
    pub release_task: Arc<ReleaseTask>,
    /// `task`, and in test builds `task_raw` (without the owner
    /// liveness).
    pub read_task: Arc<ReadTask>,
    pub block_task: Arc<BlockTask>,
    pub unblock_task: Arc<UnblockTask>,
    pub submit_task: Arc<SubmitTask>,
    pub verify_task: Arc<VerifyTask>,
    pub pause_run: Arc<PauseRun>,
    pub resume_run: Arc<ResumeRun>,
    pub resume_run_externally: Arc<ResumeRunExternally>,
    pub close_run: Arc<CloseRun>,
    pub extend_run_deadline: Arc<ExtendRunDeadline>,
    pub stop_run: Arc<StopRun>,
    pub read_control_status: Arc<ReadControlStatus>,
    pub read_usage_report: Arc<ReadUsageReport>,
    pub complete_run: Arc<CompleteRun>,
    pub revalidate_task: Arc<RevalidateTask>,
    pub amend_run_contract: Arc<AmendRunContract>,
    pub record_evidence: Arc<RecordEvidence>,
    pub configure_usage_budget: Arc<ConfigureUsageBudget>,
    pub record_request_usage: Arc<RecordRequestUsage>,
    pub read_request_admission: Arc<ReadRequestAdmission>,
    pub reserve_files: Arc<ReserveFiles>,
    pub release_files: Arc<ReleaseFiles>,
    pub list_file_owners: Arc<ListFileOwners>,
    pub recover_task: Arc<RecoverTask>,
    pub revoke_task: Arc<RevokeTask>,
    pub send_message: Arc<SendMessage>,
    pub withdraw_message: Arc<WithdrawMessage>,
    pub read_inbox: Arc<ReadInbox>,
    pub acknowledge_message: Arc<AcknowledgeMessage>,
    pub claim_notifications: Arc<ClaimNotifications>,
    pub accept_wake: Arc<AcceptWake>,
    pub quarantine_member: Arc<QuarantineMember>,
    pub confirm_member_dead: Arc<ConfirmMemberDead>,
    pub lose_coordinator: Arc<LoseCoordinator>,
    pub read_run_summary: Arc<ReadRunSummary>,
    pub read_run_events: Arc<ReadRunEvents>,
    pub list_tasks: Arc<ListTasks>,
    pub bootstrap_member: Arc<BootstrapMember>,
    pub join_member: Arc<JoinMember>,
    /// Each call's `swarm_op` record and its measure (#2303), only when
    /// the event log is switched on (`telemetry.event_log.enabled`, owner
    /// decision T1): `None` measures and writes nothing.
    pub telemetry: Option<BoardTelemetry>,
}

/// The member-wire codec (#2279), bound by composition and carried by the
/// `SwarmBoard` (constant function pointers, so reading a request never
/// resolves a board file, #2279 review L5): Python's
/// `json.loads` for a member's argument text (refused, with the reason,
/// where a `Value` cannot hold what Python read), the text field `op`
/// such a text names (read even when the rest is refused), and plain
/// `json.dumps` for an answer.
#[derive(Clone, Copy, Debug)]
pub struct BoardWire {
    pub read: fn(&str) -> Result<Value, String>,
    pub op: fn(&str) -> Option<String>,
    pub write: fn(&Value) -> Result<String, String>,
}

/// The event log a board call records in, the meter that measures it and
/// each caller's kept ref (#2303): all or none, so a record is never
/// written with waits that were not measured.
#[derive(Clone)]
pub struct BoardTelemetry {
    pub log: Arc<dyn BoardOpLog>,
    pub meter: Arc<dyn BoardCallMeter>,
    pub actors: Arc<ActorRefs>,
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
    "_event_cursor",
    "_snapshot",
    "_admit",
    "_activate",
    "_record_launch",
    "_release_unlaunched",
    "_socket",
    "task_create",
    "dependencies",
    "claim",
    "release",
    "block",
    "unblock",
    "submit",
    "verify_task",
    "pause",
    "resume",
    "_resume_external",
    "_close",
    "_extend_deadline",
    "stop",
    "_control_status",
    "usage_report",
    "complete",
    "revalidate_task",
    "amend",
    "evidence",
    "usage_budget",
    "_record_request",
    "_request_admission",
    "reserve",
    "release_files",
    "file_owners",
    "recover",
    "revoke",
    "send",
    "withdraw",
    "inbox",
    "ack",
    "_notifications",
    "_accept_wake",
    "_quarantine",
    "_confirmed_dead",
    "_lose_coordinator",
    "summary",
    "events",
    "task",
    "tasks",
    "create",
    "_bootstrap",
    "_join",
    #[cfg(any(test, feature = "test-support"))]
    "create_run",
    #[cfg(any(test, feature = "test-support"))]
    "bootstrap_run",
    #[cfg(any(test, feature = "test-support"))]
    "bootstrap_join",
    #[cfg(any(test, feature = "test-support"))]
    "task_raw",
];

/// Invokes `method` as `member` with `args`: one use case, its result in
/// Python's JSON shape. The call leaves one `tracing` record and, when the
/// event log is on, one `swarm_op` record, measured only then (#2303).
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
    call_as(handles, member, method, args, CallOrigin::Member)
}

/// [`call`], made for `origin`: a harness read on the member's behalf is
/// recorded with role `host` (#2279 S15 final review).
///
/// # Errors
/// As [`call`].
pub fn call_as(
    handles: &SwarmBoardHandles,
    member: &str,
    method: &str,
    args: Value,
    origin: CallOrigin,
) -> Result<Value, BoardError> {
    let started = Instant::now();
    let known = Method::parse(method);
    // While the event log is on: this call's own measure, which is the
    // repository it is served over, so nothing is shared with another call.
    let metered = handles
        .telemetry
        .as_ref()
        .map(|telemetry| telemetry.meter.open());
    let answer = match known {
        Some(known) => bind(known, known.parameters(), args).and_then(|arguments| {
            let over = metered
                .clone()
                .map(|metered| metered as Arc<dyn BoardRepository>);
            serve(handles, over, member, known, arguments)
        }),
        None => Err(BoardError::new(
            RefusalKind::Calling,
            format!("swarm board has no method {method}"),
        )),
    };
    let (answer, committed) = split_committed(answer);
    let measure = metered.and_then(|metered| metered.measure());
    debug_assert!(
        known.is_some()
            || measure
                .as_ref()
                .is_none_or(|measure| measure.run_roles.is_none()),
        "a name that is no board method reads no run, so no role is read from one"
    );
    let finished = Finished {
        op: known.map_or("unknown", Method::name),
        level: match (known, &answer) {
            (Some(known), Ok(served)) => known.served_level(served.decision),
            (Some(known), Err(_)) => known.level(),
            (None, _) => Level::Mutation,
        },
        role: records::recorded_role(origin, known),
        member,
        outcome: answer
            .as_ref()
            .map(|served| served.decision)
            .map_err(BoardError::kind),
        elapsed: started.elapsed(),
    };
    let caller = match (&answer, known.map(Method::answers_members_only)) {
        (Ok(_), Some(true)) => Caller::Member,
        _ => Caller::Unproven,
    };
    let served = answer.as_ref().ok().or(committed.as_ref());
    records::record(handles, &finished, caller, served, measure);
    answer.map(|served| served.value)
}

/// Binds `args` to `parameters` as Python binds a call: positionally from
/// an array, by name from an object, then each unbound parameter's default.
fn bind(method: Method, parameters: &[Parameter], args: Value) -> Result<Vec<Value>, BoardError> {
    let name = method.name();
    let mut slots: Vec<Option<Value>> = vec![None; parameters.len()];
    match args {
        Value::Array(values) => {
            if values.len() > parameters.len() {
                return Err(BoardError::new(
                    RefusalKind::Calling,
                    format!(
                        "{name}: takes {} arguments, {} given",
                        parameters.len(),
                        values.len()
                    ),
                ));
            }
            for (slot, value) in slots.iter_mut().zip(values) {
                *slot = Some(value);
            }
        }
        Value::Object(fields) => {
            for (key, value) in fields {
                let Some(index) = parameters.iter().position(|p| p.name == key) else {
                    return Err(BoardError::new(
                        RefusalKind::Calling,
                        format!("{name}: unexpected argument {key}"),
                    ));
                };
                slots[index] = Some(value);
            }
        }
        _ => {
            return Err(BoardError::new(
                RefusalKind::Calling,
                format!("{name}: arguments must be a JSON array or object"),
            ));
        }
    }
    slots
        .into_iter()
        .zip(parameters)
        .map(|(slot, parameter)| match (slot, parameter.default) {
            (Some(value), _) => Ok(value),
            (None, Some(default)) => Ok(default()),
            (None, None) => Err(BoardError::new(
                RefusalKind::Calling,
                format!("{name}: missing required argument {}", parameter.name),
            )),
        })
        .collect()
}

/// The composed use case, or the same one over a metered call's own
/// repository when there is one.
enum Serving<'a, U> {
    Composed(&'a U),
    Over(U),
}

impl<U> std::ops::Deref for Serving<'_, U> {
    type Target = U;

    fn deref(&self) -> &U {
        match self {
            Self::Composed(composed) => composed,
            Self::Over(over) => over,
        }
    }
}

fn serving<'a, U: OverRepository>(
    composed: &'a U,
    over: Option<&Arc<dyn BoardRepository>>,
) -> Serving<'a, U> {
    match over {
        Some(repository) => Serving::Over(composed.over(repository.clone())),
        None => Serving::Composed(composed),
    }
}

/// Serves `method`, over `over` (the call's metered repository) when the
/// event log is on.
fn serve(
    handles: &SwarmBoardHandles,
    over: Option<Arc<dyn BoardRepository>>,
    member: &str,
    method: Method,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    debug_assert_eq!(
        arguments.len(),
        method.parameters().len(),
        "every parameter is bound"
    );
    let over = over.as_ref();
    match method {
        Method::Status => Ok(Served {
            value: status(serving(&*handles.read_run_status, over).execute()?),
            decision: "read",
            task_id: None,
            message_id: None,
            cursor_moved: None,
            detail: BoardOpDetail::NONE,
            refused: None,
        }),
        Method::EventCursor => Ok(Served {
            value: Value::from(serving(&*handles.read_event_cursor, over).execute()?),
            decision: "read",
            task_id: None,
            message_id: None,
            cursor_moved: None,
            detail: BoardOpDetail::NONE,
            refused: None,
        }),
        Method::Snapshot => Ok(Served {
            value: snapshot(serving(&*handles.read_run_snapshot, over).execute(member)?)?,
            decision: "read",
            task_id: None,
            message_id: None,
            cursor_moved: None,
            detail: BoardOpDetail::NONE,
            refused: None,
        }),
        Method::Admit => members::admit(&serving(&*handles.admit_member, over), member, arguments),
        Method::Activate => {
            members::activate(&serving(&*handles.activate_member, over), member, arguments)
        }
        Method::RecordLaunch => members::record_launch(
            &serving(&*handles.record_member_launch, over),
            member,
            arguments,
        ),
        Method::ReleaseUnlaunched => members::release_unlaunched(
            &serving(&*handles.release_unlaunched_member, over),
            member,
            arguments,
        ),
        Method::Socket => members::socket(
            &serving(&*handles.register_member_socket, over),
            member,
            arguments,
        ),
        Method::TaskCreate => {
            tasks::task_create(&serving(&*handles.create_task, over), member, arguments)
        }
        Method::Dependencies => tasks::dependencies(
            &serving(&*handles.set_task_dependencies, over),
            member,
            arguments,
        ),
        Method::Claim => tasks::claim(&serving(&*handles.claim_task, over), member, arguments),
        Method::Release => {
            tasks::release(&serving(&*handles.release_task, over), member, arguments)
        }
        Method::Block => {
            submissions::block(&serving(&*handles.block_task, over), member, arguments)
        }
        Method::Unblock => {
            submissions::unblock(&serving(&*handles.unblock_task, over), member, arguments)
        }
        Method::Submit => {
            submissions::submit(&serving(&*handles.submit_task, over), member, arguments)
        }
        Method::VerifyTask => {
            submissions::verify_task(&serving(&*handles.verify_task, over), member, arguments)
        }
        Method::Pause => control::pause(&serving(&*handles.pause_run, over), member, arguments),
        Method::Resume => control::resume(&serving(&*handles.resume_run, over), member),
        Method::ResumeExternal => {
            control::resume_external(&serving(&*handles.resume_run_externally, over), member)
        }
        Method::Close => control::close(&serving(&*handles.close_run, over), member),
        Method::ExtendDeadline => control::extend(
            &serving(&*handles.extend_run_deadline, over),
            member,
            arguments,
        ),
        Method::Stop => control::stop(&serving(&*handles.stop_run, over), member, arguments),
        Method::ControlStatus => {
            control::control_status(&serving(&*handles.read_control_status, over), member)
        }
        Method::UsageReport => {
            control::usage_report(&serving(&*handles.read_usage_report, over), member)
        }
        Method::Complete => {
            completion::complete(&serving(&*handles.complete_run, over), member, arguments)
        }
        Method::RevalidateTask => completion::revalidate_task(
            &serving(&*handles.revalidate_task, over),
            member,
            arguments,
        ),
        Method::Amend => completion::amend(
            &serving(&*handles.amend_run_contract, over),
            member,
            arguments,
        ),
        Method::Evidence => {
            completion::evidence(&serving(&*handles.record_evidence, over), member, arguments)
        }
        Method::UsageBudget => usage::usage_budget(
            &serving(&*handles.configure_usage_budget, over),
            member,
            arguments,
        ),
        Method::RecordRequest => usage::record_request(
            &serving(&*handles.record_request_usage, over),
            member,
            arguments,
        ),
        Method::RequestAdmission => {
            usage::request_admission(&serving(&*handles.read_request_admission, over), member)
        }
        Method::Reserve => {
            reservations::reserve(&serving(&*handles.reserve_files, over), member, arguments)
        }
        Method::ReleaseFiles => {
            reservations::release_files(&serving(&*handles.release_files, over), member, arguments)
        }
        Method::FileOwners => reservations::file_owners(
            &serving(&*handles.list_file_owners, over),
            member,
            arguments,
        ),
        Method::Recover => {
            reservations::recover(&serving(&*handles.recover_task, over), member, arguments)
        }
        Method::Revoke => {
            reservations::revoke(&serving(&*handles.revoke_task, over), member, arguments)
        }
        Method::Send => messages::send(&serving(&*handles.send_message, over), member, arguments),
        Method::Withdraw => messages::withdraw(
            &serving(&*handles.withdraw_message, over),
            member,
            arguments,
        ),
        Method::Inbox => messages::inbox(&serving(&*handles.read_inbox, over), member, arguments),
        Method::Ack => messages::ack(
            &serving(&*handles.acknowledge_message, over),
            member,
            arguments,
        ),
        Method::Notifications => wakes::notifications(
            &serving(&*handles.claim_notifications, over),
            member,
            arguments,
        ),
        Method::AcceptWake => {
            wakes::accept_wake(&serving(&*handles.accept_wake, over), member, arguments)
        }
        Method::Quarantine => loss::quarantine(
            &serving(&*handles.quarantine_member, over),
            member,
            arguments,
        ),
        Method::ConfirmedDead => loss::confirmed_dead(
            &serving(&*handles.confirm_member_dead, over),
            member,
            arguments,
        ),
        Method::LoseCoordinator => {
            loss::lose_coordinator(&serving(&*handles.lose_coordinator, over), member)
        }
        Method::Summary => reads::summary(
            &serving(&*handles.read_run_summary, over),
            member,
            arguments,
        ),
        Method::Events => {
            reads::events(&serving(&*handles.read_run_events, over), member, arguments)
        }
        Method::Task => tasks::task(&serving(&*handles.read_task, over), member, arguments),
        Method::Tasks => reads::tasks(&serving(&*handles.list_tasks, over), member, arguments),
        Method::Create => reads::create(&serving(&*handles.create_run, over), member, arguments),
        Method::Bootstrap => reads::bootstrap(
            &serving(&*handles.bootstrap_member, over),
            member,
            arguments,
        ),
        Method::Join => reads::join(&serving(&*handles.join_member, over), member, arguments),
        #[cfg(any(test, feature = "test-support"))]
        Method::CreateRun => {
            test_only::create_run(&serving(&*handles.create_run, over), member, arguments)
        }
        #[cfg(any(test, feature = "test-support"))]
        Method::BootstrapRun => {
            test_only::bootstrap_run(&serving(&*handles.bootstrap_run, over), member, arguments)
        }
        #[cfg(any(test, feature = "test-support"))]
        Method::BootstrapJoin => {
            test_only::bootstrap_join(&serving(&*handles.join_run, over), member, arguments)
        }
        #[cfg(any(test, feature = "test-support"))]
        Method::TaskRaw => tasks::task_raw(&serving(&*handles.read_task, over), member, arguments),
    }
}

/// A method that returns `None` and acted on no task or message (and has
/// no cursor to move), and the decision it took.
fn done(decision: &'static str) -> Served {
    Served {
        value: Value::Null,
        decision,
        task_id: None,
        message_id: None,
        cursor_moved: None,
        detail: BoardOpDetail::NONE,
        refused: None,
    }
}

/// The bound arguments as an array of the signature's length.
fn take<const N: usize>(arguments: Vec<Value>) -> Result<[Value; N], BoardError> {
    arguments.try_into().map_err(|_| {
        BoardError::new(
            RefusalKind::Internal,
            "swarm board bound the wrong number of arguments",
        )
    })
}

#[path = "swarm_board_dispatch_records.rs"]
mod records;
pub use records::{CallOrigin, refused, signature};

#[path = "swarm_board_dispatch_render.rs"]
mod render;
use render::{float, member_row, object, snapshot, status};

#[path = "swarm_board_dispatch_members.rs"]
mod members;

#[path = "swarm_board_dispatch_method.rs"]
mod method;

#[path = "swarm_board_dispatch_tasks.rs"]
mod tasks;

#[path = "swarm_board_dispatch_submissions.rs"]
mod submissions;

/// The test-only methods: their signatures and their serving.
#[cfg(any(test, feature = "test-support"))]
#[path = "swarm_board_dispatch_test_only.rs"]
mod test_only;

#[path = "swarm_board_dispatch_control.rs"]
mod control;

#[path = "swarm_board_dispatch_completion.rs"]
mod completion;

#[path = "swarm_board_dispatch_reservations.rs"]
mod reservations;

#[path = "swarm_board_dispatch_loss.rs"]
mod loss;
#[path = "swarm_board_dispatch_messages.rs"]
mod messages;
#[path = "swarm_board_dispatch_reads.rs"]
mod reads;
#[path = "swarm_board_dispatch_usage.rs"]
mod usage;
#[path = "swarm_board_dispatch_wakes.rs"]
mod wakes;

#[cfg(test)]
#[path = "swarm_board_dispatch_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "swarm_board_dispatch_telemetry_tests.rs"]
mod telemetry_tests;

#[cfg(test)]
#[path = "swarm_board_dispatch_stall_tests.rs"]
mod stall_tests;

#[cfg(test)]
#[path = "swarm_board_dispatch_loss_tests.rs"]
mod loss_tests;

#[cfg(test)]
#[path = "swarm_board_dispatch_reads_tests.rs"]
mod reads_tests;
