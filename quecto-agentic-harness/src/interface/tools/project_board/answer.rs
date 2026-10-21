//! A board operation's answer as its front ends send it: one JSON object
//! of a stable shape.
//!
//! - Success: `{"ok": true, "op": …}` plus what the op answers: `task` (and
//!   after a change that may complete it, `completion`: `{"done": true,
//!   "task"}`, `{"done": false, "pending": [prs]}` while some have not
//!   merged, or `{"done": false, "error"}`), a listing's `head`, `tasks`
//!   and `quarantined` (asked for summaries, each task with its `summary`,
//!   [`listing_fields`], or `summary_skipped` and none), `show`'s
//!   ([`shown_fields`]), a plan proposed,
//!   edited or discarded as `plan` ([`planned_json`]), a plan accepted as
//!   the `task` naming it, the `plan` and the version it `superseded`
//!   (`null` when none was accepted), or `head` alone; and
//!   `clock` when a request that reads the clock was not bounded against
//!   an outside time. On the agent tool, a started task's answer also
//!   carries `steps` ([`start_steps`]).
//! - Refusal: `{"ok": false, "op": …, "error": {"code", "message", …}}`.
//!   Each [`ErrorCode`] has one shape: the same fields every time, but
//!   a refused start's `steps` on the agent tool ([`start_steps`]), and
//!   `artifact_set_aside` and `duplicate_artifact`, which name an
//!   artifact by its kind: a discussion by `discussion` where a plan, run,
//!   change or review gives `task` and `number` (and a task, for
//!   `duplicate_artifact`, `task` alone).
//!   Messages are the error's own words (its `Display`); every text an
//!   answer carries from outside (messages, reasons, fields, problems,
//!   paths) has credentials redacted and is at most [`MAX_MESSAGE_CHARS`]
//!   characters.
//!
//! A set-aside file an answer lists (`quarantined_json`) lies directly
//! under `tasks/` when it is a task's (`shown_path`), under its own kind's
//! folder when it is another kind's ([`shown_artifact_path`], named by its
//! key as `artifact_set_aside` names one, #2481 M4a), and a stray or a
//! carried file of no known artifact (`set_aside_file`) directly under any
//! folder the store reads ([`shown_board_path`], S9).
//!
//! A task's shape is the board's own field names, every field always
//! present (`null` or `[]` when unset), in a fixed order.
use serde_json::{Map, Value, json};

use super::request::RequestError;
use crate::application::project_board::dto::{LivenessCheck, ProjectBoardError, RunReport};
use crate::application::project_board::operations::{BoardOutcome, Completion};
use crate::application::project_board::ports::{
    ArtifactKey, BoardClockError, BoardHead, BoardStoreError, PullRequestStatesError,
    QuarantinedFile,
};
use crate::application::project_board::start::StartedTask;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::shown_text::{
    shown_artifact_path, shown_board_path,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::RunOutcome;
use crate::domain::shared::redaction::{redact_secrets, redact_url_userinfo};

#[path = "answer_artifacts.rs"]
mod artifacts;
use artifacts::{
    artifact_folder, listing_fields, page_fields, planned_json, quarantined_json, run_json,
    shown_fields,
};

/// The most characters of an error's message an answer carries.
pub const MAX_MESSAGE_CHARS: usize = 2_000;

/// Every refusal an answer may carry, each with its one shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// `field`, `problem`.
    InvalidRequest,
    /// `reason`: the repository names no committer to write the board as.
    NoCommitter,
    NoBoard,
    /// `reason`.
    Clash,
    /// `reason`.
    InvalidWrite,
    /// `reason`.
    StoreUnavailable,
    /// `reason`: the board is past what a read takes.
    BoardTooLarge,
    /// `reason`: the board is held outside `tasks/`: no listing while a
    /// file set aside there names another kind's key.
    FolderSetAside,
    /// `rule`: `{"code", …}`.
    Rule,
    /// `field`, `problem`.
    InvalidTask,
    /// `field`, `problem`.
    InvalidWriter,
    /// `task`.
    NotFound,
    /// `task`.
    Exists,
    /// `task` (the id asked for) and `by`: the artifact already naming it,
    /// as `duplicate_artifact` names one.
    IdInUse,
    /// `read`.
    ClockUnreadable,
    /// `task`.
    SetAside,
    /// A file of an artifact other than a task is set aside: `kind`
    /// (`plan`, `run`, `change` or `review`), `task`, `number` (the plan's
    /// version, the run's or change's number, the reviewed change's) and
    /// `path`; for a discussion, `kind`, `discussion` (its id) and `path`.
    ArtifactSetAside,
    /// The board holds an artifact more than once (its store broke its
    /// contract of one artifact per key), named as `artifact_set_aside`
    /// names one, without a path: `kind`, `task`, `number` for a plan,
    /// run, change or review; `kind`, `discussion` for a discussion;
    /// `kind`, `task` for a task.
    DuplicateArtifact,
    /// `path`, `object`.
    SetAsideFile,
    /// `task`.
    UnknownReference,
    /// `task`, `by`.
    ClosesCycle,
    /// `pr`.
    PrStateUnknown,
    /// `reason`.
    PullRequestsUnavailable,
    /// `task`, `updated`.
    TaskAhead,
    /// `reference`.
    ClockOutOfRange,
    /// `now`, `reference`.
    ClockAhead,
    /// `attempts`.
    Contended,
    /// `reason`.
    Refused,
    /// `task`, `expected`, `updated`.
    Stale,
    /// `path`, `object`.
    NotSetAside,
    /// `reason`: the board's remote reports no pull requests.
    NoPullRequestHost,
    /// This session may not spawn, so it starts nothing (S4c).
    NoLauncher,
    /// `task`, `run`: the run's swarm still runs here, so it is not ended
    /// by hand (S4c).
    RunLive,
    /// `brief`: `{"code"}`, why the plan cannot brief a swarm (S4c).
    Brief,
    /// `reason` (the launch's), `run`, `run_state` (`folded`,
    /// `ended_already`, or `open`: for `end_run`), `released` (`null` when
    /// the start did not take the claim), `restored` (`null`, or the review
    /// or blocked `status` the task was held in and whether it `moved`
    /// back), `still_running` (`null`, or the coordinator that may still
    /// run: nothing was undone): the coordinator did not launch,
    /// and the start was undone as far as it could be (S4c).
    LaunchFailed,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::NoCommitter => "no_committer",
            Self::NoBoard => "no_board",
            Self::Clash => "clash",
            Self::InvalidWrite => "invalid_write",
            Self::StoreUnavailable => "store_unavailable",
            Self::BoardTooLarge => "board_too_large",
            Self::FolderSetAside => "folder_set_aside",
            Self::Rule => "rule",
            Self::InvalidTask => "invalid_task",
            Self::InvalidWriter => "invalid_writer",
            Self::NotFound => "not_found",
            Self::Exists => "exists",
            Self::IdInUse => "id_in_use",
            Self::ClockUnreadable => "clock_unreadable",
            Self::SetAside => "set_aside",
            Self::ArtifactSetAside => "artifact_set_aside",
            Self::DuplicateArtifact => "duplicate_artifact",
            Self::SetAsideFile => "set_aside_file",
            Self::UnknownReference => "unknown_reference",
            Self::ClosesCycle => "closes_cycle",
            Self::PrStateUnknown => "pr_state_unknown",
            Self::PullRequestsUnavailable => "pull_requests_unavailable",
            Self::TaskAhead => "task_ahead",
            Self::ClockOutOfRange => "clock_out_of_range",
            Self::ClockAhead => "clock_ahead",
            Self::Contended => "contended",
            Self::Refused => "refused",
            Self::Stale => "stale",
            Self::NotSetAside => "not_set_aside",
            Self::NoPullRequestHost => "no_pull_request_host",
            Self::NoLauncher => "no_launcher",
            Self::RunLive => "run_live",
            Self::Brief => "brief",
            Self::LaunchFailed => "launch_failed",
        }
    }
}

/// Text an answer may carry: credentials in URLs and every secret-shaped
/// span redacted, bounded.
pub fn shown(text: &str) -> String {
    let redacted = redact_secrets(&redact_url_userinfo(text));
    match redacted.chars().count() > MAX_MESSAGE_CHARS {
        true => redacted.chars().take(MAX_MESSAGE_CHARS).collect::<String>() + "…",
        false => redacted,
    }
}

fn shown_json(text: &str) -> Value {
    Value::String(shown(text))
}

/// The answer to `op`, which came to `outcome`; `clock` says why its
/// changes were not bounded against an outside time.
/// What an `end_run` by its holder recorded of the run's task items when
/// no swarm reported them (S4c): said only then.
const NO_SWARM_REPORT: &str = "items recorded as todo: no swarm report";

/// What an `end_run` from a front end that cannot see running swarms (the
/// CLI) leaves its caller to have made sure of (S4c).
const NOT_CHECKED: &str =
    "this end was not checked against running swarms: if the run's coordinator still runs, stop it";

/// `steps`, when given, are the steps the caller takes next (a start's
/// on the agent tool, [`start_steps`]).
pub fn outcome(
    op: &str,
    outcome: &BoardOutcome,
    clock: Option<String>,
    steps: Option<String>,
) -> Value {
    let mut fields = Map::new();
    match outcome {
        BoardOutcome::Bootstrapped { head, created } => {
            fields.insert("head".into(), head_json(head));
            fields.insert("created".into(), Value::Bool(*created));
        }
        BoardOutcome::Listing(listing) => fields.extend(listing_fields(listing)),
        BoardOutcome::Shown(shown, view) => match view.page {
            None => fields.extend(shown_fields(shown, view.version)),
            Some(page) => fields.extend(page_fields(shown, page, view.version)),
        },
        BoardOutcome::Changed { task, completion } => {
            fields.insert("task".into(), task_json(task));
            if let Some(completion) = completion {
                let completion = match completion {
                    Completion::Done(task) => json!({"done": true, "task": task_json(task)}),
                    Completion::Pending(prs) => json!({"done": false, "pending": prs}),
                    Completion::Failed(error) => {
                        json!({"done": false, "error": error_json(error)})
                    }
                };
                fields.insert("completion".into(), completion);
            }
        }
        BoardOutcome::Cleared { head } => {
            fields.insert("head".into(), head_json(head));
        }
        BoardOutcome::Planned(plan) => {
            fields.insert("plan".into(), planned_json(plan));
        }
        BoardOutcome::RunEnded(ended) => {
            let run = &ended.run;
            fields.insert("run".into(), run_json(run));
            // Whether this front end could see the run's swarm still
            // running; one that could not says what that leaves its caller.
            fields.insert("liveness".into(), json!(ended.liveness.as_str()));
            match ended.liveness {
                LivenessCheck::Checked => {}
                LivenessCheck::NotChecked => {
                    fields.insert("liveness_note".into(), json!(NOT_CHECKED));
                }
            }
            // An end no swarm reported: what it recorded of the items. One
            // from its swarm's kept board shows them in the run itself.
            let report = match (run.fields().outcome, ended.report) {
                (
                    Some(RunOutcome::Succeeded | RunOutcome::Stopped | RunOutcome::Failed),
                    RunReport::Unreported,
                ) => Some(NO_SWARM_REPORT),
                (Some(RunOutcome::Lost) | None, _) | (_, RunReport::Kept) => None,
            };
            if let Some(report) = report {
                fields.insert("items_recorded".into(), json!(report));
            }
        }
        BoardOutcome::Started(started) => fields.extend(started_fields(started)),
        BoardOutcome::Accepted(accepted) => {
            fields.insert("task".into(), task_json(&accepted.task));
            fields.insert("plan".into(), planned_json(&accepted.plan));
            let superseded = accepted.superseded.as_ref();
            let superseded = superseded.map(|plan| plan.fields().version);
            fields.insert("superseded".into(), json!(superseded));
        }
    }
    if let Some(why) = clock {
        fields.insert(
            "clock".into(),
            json!({"referenced": false, "reason": shown(&why)}),
        );
    }
    if let Some(steps) = steps {
        fields.insert("steps".into(), Value::String(steps));
    }
    let mut answer = Map::new();
    answer.insert("ok".into(), Value::Bool(true));
    answer.insert("op".into(), Value::String(op.into()));
    answer.extend(fields);
    Value::Object(answer)
}

/// What `start` answers (S4c): the `task` as written, its `run`, the
/// `coordinator` launched for it (its `agent` and the container it runs
/// in, `environment_ref`), the worker roles whose models were not applied
/// (`not_applied`: `role`, `model`, `effort`), whether the start took
/// the claim (`claim`: `fresh`) or found it held by the starter (`held`);
/// on the agent tool, [`outcome`] adds the `steps` for running it
/// ([`start_steps::running`]).
fn started_fields(started: &StartedTask) -> Map<String, Value> {
    let not_applied: Vec<Value> = started
        .not_applied
        .iter()
        .map(|role| {
            json!({
                "role": slug_json(&role.role),
                "model": shown(&role.model),
                "effort": role.effort.as_str(),
            })
        })
        .collect();
    let coordinator = &started.coordinator;
    fields([
        ("task", task_json(&started.task)),
        ("run", run_json(&started.run)),
        (
            "coordinator",
            json!({
                "agent": shown(&coordinator.agent),
                "environment_ref": shown(&coordinator.environment_ref),
            }),
        ),
        ("not_applied", Value::Array(not_applied)),
        ("claim", json!(started.claim.as_str())),
    ])
}

/// The files set aside, each with its `object` and the `copies` (every
/// distinct object listed at its path, itself included): what a clear
/// names.
fn quarantined_list(files: &[QuarantinedFile]) -> Value {
    Value::Array(
        files
            .iter()
            .map(|file| {
                let copies: std::collections::BTreeSet<&str> = files
                    .iter()
                    .filter(|other| other.path == file.path)
                    .map(|other| other.object.as_str())
                    .collect();
                let mut entry = quarantined_json(file);
                entry["copies"] = json!(copies);
                entry
            })
            .collect(),
    )
}

fn head_json(head: &BoardHead) -> Value {
    Value::String(head.0.clone())
}

fn slug_json(slug: &Slug) -> Value {
    Value::String(slug.as_str().into())
}

/// The artifact a refusal names: a task as `set_aside` by its id, as
/// ever; any other kind as `artifact_set_aside` by [`key_fields`] and its
/// file's raw path, so a clear can name it exactly.
fn set_aside_fields(
    key: &ArtifactKey,
    path: &str,
    object: &str,
) -> (ErrorCode, Map<String, Value>) {
    assert!(
        object.bytes().all(|b| b.is_ascii_hexdigit()),
        "a set-aside file's object is git's id"
    );
    let folder = match key {
        ArtifactKey::Task(id) => {
            return (
                ErrorCode::SetAside,
                fields([("task", slug_json(id)), ("object", json!(object))]),
            );
        }
        ArtifactKey::Plan(..)
        | ArtifactKey::Run(..)
        | ArtifactKey::Change(..)
        | ArtifactKey::Review(..)
        | ArtifactKey::Discussion(_) => {
            artifact_folder(key).expect("every kind but a task has a folder")
        }
    };
    // Its own kind's folder, one name in it.
    assert!(
        shown_artifact_path(folder, path),
        "a store shows every path it sets aside sanitised: {path:?}"
    );
    let mut named = key_fields(key);
    named.insert("path".into(), json!(path));
    named.insert("object".into(), json!(object));
    (ErrorCode::ArtifactSetAside, named)
}

/// An artifact by its key: `kind` and `task` for a task; `kind`, `task`
/// and `number` (the plan's version, the run's or change's number, the
/// reviewed change's) for a plan, run, change or review; `kind` and
/// `discussion` (its id) for a discussion.
fn key_fields(key: &ArtifactKey) -> Map<String, Value> {
    let numbered = |kind: &str, task: &Slug, number: u32| {
        fields([
            ("kind", json!(kind)),
            ("task", slug_json(task)),
            ("number", json!(number)),
        ])
    };
    match key {
        ArtifactKey::Task(id) => fields([("kind", json!("task")), ("task", slug_json(id))]),
        ArtifactKey::Plan(task, version) => numbered("plan", task, *version),
        ArtifactKey::Run(task, number) => numbered("run", task, *number),
        ArtifactKey::Change(task, number) => numbered("change", task, *number),
        ArtifactKey::Review(task, change) => numbered("review", task, *change),
        ArtifactKey::Discussion(id) => {
            fields([("kind", json!("discussion")), ("discussion", slug_json(id))])
        }
    }
}

fn slugs_json(slugs: &[Slug]) -> Value {
    Value::Array(slugs.iter().map(slug_json).collect())
}

fn identity_json(identity: &Identity) -> Value {
    json!({"name": identity.name, "email": identity.email})
}

/// A task in the board's own field names, every field present but where
/// it came from and how it ended (`github_issue`, `resolution`,
/// `superseded_by`, `references`): those are held back on purpose, as
/// showing them to agents is the owner's call (M4).
pub fn task_json(task: &Task) -> Value {
    let task = task.fields();
    json!({
        "id": slug_json(&task.id),
        "title": task.title,
        "kind": task.kind.as_str(),
        "description": task.description,
        "status": task.status.as_str(),
        "parent": task.parent.as_ref().map(slug_json),
        "depends_on": slugs_json(&task.depends_on),
        "accepted_plan": task.accepted_plan,
        "claim": task.claim.as_ref().map(|claim| json!({
            "holder": identity_json(&claim.holder),
            "since": claim.since.as_str(),
            "expires": claim.expires.as_str(),
        })),
        "prs": task.prs,
        "created": task.created.as_str(),
        "updated": task.updated.as_str(),
    })
}

/// A refused answer to `op` (`None` when the request named no op it
/// knows), with `code`'s own fields and the `steps` its caller takes next,
/// when it has any.
fn refused(
    op: Option<&str>,
    code: ErrorCode,
    message: &str,
    fields: Map<String, Value>,
    steps: Option<&str>,
) -> Value {
    let mut error = Map::new();
    error.insert("code".into(), Value::String(code.as_str().into()));
    error.insert("message".into(), shown_json(message));
    error.extend(fields);
    if let Some(steps) = steps {
        error.insert("steps".into(), json!(steps));
    }
    json!({"ok": false, "op": op, "error": Value::Object(error)})
}

/// A request refused before the board was read, with `steps` as
/// [`refused`] takes them.
pub fn request_refused(op: Option<&str>, error: &RequestError, steps: Option<&str>) -> Value {
    refused(
        op,
        ErrorCode::InvalidRequest,
        &format!("{}: {}", error.field, error.problem),
        fields([
            ("field", shown_json(&error.field)),
            ("problem", shown_json(&error.problem)),
        ]),
        steps,
    )
}

/// The refusals a front end makes itself, before the board answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrontEndRefusal {
    /// A write, and the repository names no committer.
    NoCommitter,
    /// The board's store could not be built, or is not composed.
    StoreUnavailable,
}

impl FrontEndRefusal {
    /// The code its answer carries.
    pub fn code(self) -> ErrorCode {
        match self {
            Self::NoCommitter => ErrorCode::NoCommitter,
            Self::StoreUnavailable => ErrorCode::StoreUnavailable,
        }
    }
}

/// A front end's own refusal with a `reason`, with `steps` as [`refused`]
/// takes them.
pub fn reason_refused(
    op: Option<&str>,
    refusal: FrontEndRefusal,
    reason: &str,
    steps: Option<&str>,
) -> Value {
    let fields = fields([("reason", shown_json(reason))]);
    refused(op, refusal.code(), reason, fields, steps)
}

/// An operation the board refused, its error carrying the `steps` its
/// caller takes next, when it has any (a start's, [`start_steps`]).
pub fn board_refused_with(op: &str, error: &ProjectBoardError, steps: Option<String>) -> Value {
    let mut error = error_json(error);
    if let Some(steps) = steps {
        error["steps"] = Value::String(steps);
    }
    json!({"ok": false, "op": op, "error": error})
}

/// The code an answer refusing `error` carries.
pub fn error_code(error: &ProjectBoardError) -> ErrorCode {
    error_fields(error).0
}

fn error_json(error: &ProjectBoardError) -> Value {
    let (code, fields) = error_fields(error);
    let mut object = Map::new();
    object.insert("code".into(), Value::String(code.as_str().into()));
    object.insert("message".into(), shown_json(&error.to_string()));
    object.extend(fields);
    Value::Object(object)
}

fn fields<const N: usize>(pairs: [(&'static str, Value); N]) -> Map<String, Value> {
    pairs
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn error_fields(error: &ProjectBoardError) -> (ErrorCode, Map<String, Value>) {
    use ErrorCode as C;
    use ProjectBoardError as E;
    match error {
        E::Store(BoardStoreError::NoBoard) => (C::NoBoard, Map::new()),
        E::Store(BoardStoreError::Clash(why)) => (C::Clash, fields([("reason", shown_json(why))])),
        // Past the board's bytes is answered as an invalid write.
        E::Store(BoardStoreError::InvalidWrite(why) | BoardStoreError::WriteTooLarge(why)) => {
            (C::InvalidWrite, fields([("reason", shown_json(why))]))
        }
        E::Store(BoardStoreError::Unavailable(why)) => {
            (C::StoreUnavailable, fields([("reason", shown_json(why))]))
        }
        E::Store(BoardStoreError::TooLarge(why)) => {
            (C::BoardTooLarge, fields([("reason", shown_json(why))]))
        }
        E::Store(BoardStoreError::FolderSetAside { why, path, object }) => (
            C::FolderSetAside,
            fields([
                ("reason", shown_json(why)),
                ("path", shown_json(path)),
                ("object", json!(object)),
            ]),
        ),
        E::Rule(rule) => (C::Rule, fields([("rule", rule_json(rule))])),
        E::Invalid(schema) => (
            C::InvalidTask,
            fields([
                ("field", shown_json(&schema.field)),
                ("problem", shown_json(&schema.problem)),
            ]),
        ),
        E::InvalidWriter(schema) => (
            C::InvalidWriter,
            fields([
                ("field", shown_json(&schema.field)),
                ("problem", shown_json(&schema.problem)),
            ]),
        ),
        E::NotFound(id) => (C::NotFound, fields([("task", slug_json(id))])),
        E::Exists(id) => (C::Exists, fields([("task", slug_json(id))])),
        E::IdInUse { id, by } => (
            C::IdInUse,
            fields([
                ("task", slug_json(id)),
                ("by", Value::Object(key_fields(&by.key(id)))),
            ]),
        ),
        E::Clock(BoardClockError::OutOfRange(read)) => {
            (C::ClockUnreadable, fields([("read", shown_json(read))]))
        }
        E::Quarantined { key, path, object } => set_aside_fields(key, path, object),
        E::Duplicate { key } => (C::DuplicateArtifact, key_fields(key)),
        E::SetAsideFile { path, object } => {
            assert!(
                shown_board_path(path),
                "a store shows every path it sets aside sanitised: {path:?}"
            );
            // Raw, as a listing shows it: a clear names the file exactly.
            (
                C::SetAsideFile,
                fields([("path", json!(path)), ("object", json!(object))]),
            )
        }
        E::UnknownReference(id) => (C::UnknownReference, fields([("task", slug_json(id))])),
        E::ClosesCycle { id, by } => (
            C::ClosesCycle,
            fields([("task", slug_json(id)), ("by", slug_json(by))]),
        ),
        E::PrStateUnknown(pr) => (C::PrStateUnknown, fields([("pr", json!(pr))])),
        E::PullRequests(PullRequestStatesError::Unavailable(why)) => (
            C::PullRequestsUnavailable,
            fields([("reason", shown_json(why))]),
        ),
        E::TaskAhead { id, updated } => (
            C::TaskAhead,
            fields([
                ("task", slug_json(id)),
                ("updated", json!(updated.as_str())),
            ]),
        ),
        E::ClockOutOfRange(at) => (
            C::ClockOutOfRange,
            fields([("reference", json!(at.as_str()))]),
        ),
        E::ClockAhead { now, reference } => (
            C::ClockAhead,
            fields([
                ("now", json!(now.as_str())),
                ("reference", json!(reference.as_str())),
            ]),
        ),
        E::Contended { attempts } => (C::Contended, fields([("attempts", json!(attempts))])),
        E::Refused(why) => (C::Refused, fields([("reason", shown_json(why))])),
        E::Stale {
            id,
            expected,
            updated,
        } => (
            C::Stale,
            fields([
                ("task", slug_json(id)),
                ("expected", json!(expected.as_str())),
                ("updated", json!(updated.as_str())),
            ]),
        ),
        E::NoPullRequestHost(why) => (
            C::NoPullRequestHost,
            fields([("reason", shown_json(&why.to_string()))]),
        ),
        E::NoLauncher => (C::NoLauncher, Map::new()),
        E::RunLive { task, run, agent } => (
            C::RunLive,
            fields([
                ("task", slug_json(task)),
                ("run", json!(run)),
                ("agent", shown_json(agent)),
            ]),
        ),
        E::Brief { code, .. } => (C::Brief, fields([("brief", json!({"code": code}))])),
        E::LaunchFailed {
            reason,
            run,
            run_state,
            released,
            restored,
            still_running,
        } => (
            C::LaunchFailed,
            fields([
                ("reason", shown_json(reason)),
                ("run", json!(run)),
                ("run_state", json!(run_state.as_str())),
                ("released", json!(released)),
                ("still_running", json!(still_running.as_deref().map(shown))),
                (
                    "restored",
                    json!(restored.map(|restored| json!({
                        "status": restored.status.as_str(),
                        "moved": restored.moved,
                    }))),
                ),
            ]),
        ),
        E::NotSetAside { path, object } => (
            C::NotSetAside,
            fields([("path", shown_json(path)), ("object", shown_json(object))]),
        ),
    }
}

#[path = "answer_start.rs"]
pub mod start_steps;

#[path = "answer_rules.rs"]
mod rules;
pub use rules::rule_code;
use rules::rule_json;

#[cfg(test)]
#[path = "answer_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "answer_show_tests.rs"]
mod show_tests;
