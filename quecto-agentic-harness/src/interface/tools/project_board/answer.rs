//! A board operation's answer as its front ends send it: one JSON object
//! of a stable shape.
//!
//! - Success: `{"ok": true, "op": …}` plus what the op answers: `task` (and
//!   after a change that may complete it, `completion`: `{"done": true,
//!   "task"}`, `{"done": false, "pending": [prs]}` while some have not
//!   merged, or `{"done": false, "error"}`), a listing's `head`, `tasks`
//!   and `quarantined`, `show`'s ([`shown_fields`]), or `head` alone; and
//!   `clock` when a request that reads the clock was not bounded against
//!   an outside time.
//! - Refusal: `{"ok": false, "op": …, "error": {"code", "message", …}}`.
//!   Each [`ErrorCode`] has one shape: the same fields every time, but
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
use crate::application::project_board::dto::{BoardListing, ProjectBoardError};
use crate::application::project_board::operations::{BoardOutcome, Completion};
use crate::application::project_board::ports::{
    ArtifactKey, BoardClockError, BoardHead, BoardStoreError, PullRequestStatesError,
    QuarantinedFile,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::shown_text::{
    shown_artifact_path, shown_board_path,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::redaction::{redact_secrets, redact_url_userinfo};

#[path = "answer_artifacts.rs"]
mod artifacts;
use artifacts::{artifact_folder, page_fields, quarantined_json, shown_fields};

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
    /// `reason`: a file set aside outside `tasks/` holds the board (no
    /// write or clear, and no listing while it names another kind's key).
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
pub fn outcome(op: &str, outcome: &BoardOutcome, clock: Option<String>) -> Value {
    let mut fields = Map::new();
    match outcome {
        BoardOutcome::Bootstrapped { head, created } => {
            fields.insert("head".into(), head_json(head));
            fields.insert("created".into(), Value::Bool(*created));
        }
        BoardOutcome::Listing(listing) => fields.extend(listing_fields(listing)),
        BoardOutcome::Shown(shown, None) => fields.extend(shown_fields(shown)),
        BoardOutcome::Shown(shown, Some(page)) => fields.extend(page_fields(shown, *page)),
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
    }
    if let Some(why) = clock {
        fields.insert(
            "clock".into(),
            json!({"referenced": false, "reason": shown(&why)}),
        );
    }
    let mut answer = Map::new();
    answer.insert("ok".into(), Value::Bool(true));
    answer.insert("op".into(), Value::String(op.into()));
    answer.extend(fields);
    Value::Object(answer)
}

fn listing_fields(listing: &BoardListing) -> Map<String, Value> {
    Map::from_iter([
        ("head".to_string(), head_json(&listing.head)),
        (
            "tasks".to_string(),
            Value::Array(listing.tasks.iter().map(task_json).collect()),
        ),
        (
            "quarantined".to_string(),
            quarantined_list(&listing.quarantined),
        ),
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
fn set_aside_fields(key: &ArtifactKey, path: &str) -> (ErrorCode, Map<String, Value>) {
    let folder = match key {
        ArtifactKey::Task(id) => {
            return (ErrorCode::SetAside, fields([("task", slug_json(id))]));
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
/// knows), with `code`'s own fields.
fn refused(op: Option<&str>, code: ErrorCode, message: &str, fields: Map<String, Value>) -> Value {
    let mut error = Map::new();
    error.insert("code".into(), Value::String(code.as_str().into()));
    error.insert("message".into(), shown_json(message));
    error.extend(fields);
    json!({"ok": false, "op": op, "error": Value::Object(error)})
}

/// A request refused before the board was read.
pub fn request_refused(op: Option<&str>, error: &RequestError) -> Value {
    refused(
        op,
        ErrorCode::InvalidRequest,
        &format!("{}: {}", error.field, error.problem),
        fields([
            ("field", shown_json(&error.field)),
            ("problem", shown_json(&error.problem)),
        ]),
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

/// A front end's own refusal with a `reason`.
pub fn reason_refused(op: Option<&str>, refusal: FrontEndRefusal, reason: &str) -> Value {
    let fields = fields([("reason", shown_json(reason))]);
    refused(op, refusal.code(), reason, fields)
}

/// An operation the board refused.
pub fn board_refused(op: &str, error: &ProjectBoardError) -> Value {
    json!({"ok": false, "op": op, "error": error_json(error)})
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
        E::Store(BoardStoreError::InvalidWrite(why)) => {
            (C::InvalidWrite, fields([("reason", shown_json(why))]))
        }
        E::Store(BoardStoreError::Unavailable(why)) => {
            (C::StoreUnavailable, fields([("reason", shown_json(why))]))
        }
        E::Store(BoardStoreError::TooLarge(why)) => {
            (C::BoardTooLarge, fields([("reason", shown_json(why))]))
        }
        E::Store(BoardStoreError::FolderSetAside(why)) => {
            (C::FolderSetAside, fields([("reason", shown_json(why))]))
        }
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
        E::Quarantined { key, path } => set_aside_fields(key, path),
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
        E::NotSetAside { path, object } => (
            C::NotSetAside,
            fields([("path", shown_json(path)), ("object", shown_json(object))]),
        ),
    }
}

/// A board rule's refusal: `{"code": …}` and the rule's own fields.
fn rule_json(rule: &BoardRuleError) -> Value {
    let mut answer = Map::new();
    answer.insert("code".into(), Value::String(rule_code(rule).into()));
    answer.extend(rule_fields(rule));
    Value::Object(answer)
}

/// A board rule's code, as its refusal answers it and a call's record
/// holds it (epic #2481 T3). Exhaustive, so a new rule does not compile
/// until it has its code.
pub fn rule_code(rule: &BoardRuleError) -> &'static str {
    use BoardRuleError as R;
    match rule {
        R::NotAllowed { .. } => "not_allowed",
        R::NotHolder => "not_holder",
        R::NoClaim => "no_claim",
        R::ClaimLive => "claim_live",
        R::ClaimExpired => "claim_expired",
        R::RunOpen(..) => "run_open",
        R::RunEnded(..) => "run_ended",
        R::PlanNotDraft(..) => "plan_not_draft",
        R::NotThisTasks => "not_this_tasks",
        R::NotThisChange => "not_this_change",
        R::TaskFinished(..) => "task_finished",
        R::UnknownPlanVersion(..) => "unknown_plan_version",
        R::PlanChanged { .. } => "plan_changed",
        R::PlansInconsistent(..) => "plans_inconsistent",
        R::PlanAhead { .. } => "plan_ahead",
        R::RunsInconsistent(..) => "runs_inconsistent",
        R::PlanNotAccepted(..) => "plan_not_accepted",
        R::ChangeAlreadyLinked { .. } => "change_already_linked",
        R::NoSuchChange(..) => "no_such_change",
        R::ChangeAhead { .. } => "change_ahead",
        R::DiscussionClosed(..) => "discussion_closed",
        R::DiscussionLinked { .. } => "discussion_linked",
        R::DiscussionAhead { .. } => "discussion_ahead",
        R::MessageAhead { .. } => "message_ahead",
        R::DiscussionFull { .. } => "discussion_full",
        R::ReviewAhead { .. } => "review_ahead",
        R::ChangesLinked(..) => "changes_linked",
        R::PrUnread(..) => "pr_unread",
        R::PrNotOpen(..) => "pr_not_open",
        R::PrHeadMismatch(..) => "pr_head_mismatch",
        R::ChangeUnlinked(..) => "change_unlinked",
        R::ChangeMerged(..) => "change_merged",
        R::NoChanges => "no_changes",
        R::ChangesUnmerged(..) => "changes_unmerged",
        R::ChangesInconsistent(..) => "changes_inconsistent",
        R::NotTakeable(..) => "not_takeable",
        R::AlreadyHolder => "already_holder",
        R::NoPrs => "no_prs",
        R::PrsUnmerged(..) => "prs_unmerged",
        R::PrsLinked(..) => "prs_linked",
        R::PrNotLinked(..) => "pr_not_linked",
        R::PrAlreadyLinked(..) => "pr_already_linked",
        R::NotDraft(..) => "not_draft",
        R::NoEdit => "no_edit",
        R::Superseded { .. } => "superseded",
        R::IssueShared { .. } => "issue_shared",
        R::IssueIsPr { .. } => "issue_is_pr",
        R::OwnPrReferenced { .. } => "own_pr_referenced",
        R::Invalid(..) => "invalid",
    }
}

/// A JSON object's fields, in order: `object!({"key": value, ...})`, each
/// value as `json!` reads it.
macro_rules! object {
    ({$($key:literal: $value:expr),* $(,)?}) => {{
        let mut fields = Map::new();
        $(fields.insert($key.to_string(), json!($value));)*
        fields
    }};
}

/// A board rule's own fields, its code aside.
fn rule_fields(rule: &BoardRuleError) -> Map<String, Value> {
    use BoardRuleError as R;
    match rule {
        R::NotAllowed { from, to } => {
            object!({"from": from.as_str(), "to": to.as_str()})
        }
        R::NotHolder => Map::new(),
        R::NoClaim => Map::new(),
        R::ClaimLive => Map::new(),
        R::ClaimExpired => Map::new(),
        R::RunOpen(run) => object!({"run": run}),
        R::RunEnded(run) => object!({"run": run}),
        R::PlanNotDraft(state) => object!({"state": state.as_str()}),
        R::NotThisTasks => Map::new(),
        R::NotThisChange => Map::new(),
        R::TaskFinished(status) => object!({"status": status.as_str()}),
        R::UnknownPlanVersion(version) => {
            object!({"version": version})
        }
        R::PlanChanged { version, updated } => object!({
            "version": version,
            "updated": updated.as_str(),
        }),
        R::PlansInconsistent(why) => object!({"reason": shown(why)}),
        R::PlanAhead { version, updated } => object!({
            "version": version,
            "updated": updated.as_str(),
        }),
        R::RunsInconsistent(why) => object!({"reason": shown(why)}),
        R::PlanNotAccepted(version) => object!({"version": version}),
        R::ChangeAlreadyLinked { task, change } => {
            object!({"task": task.as_str(), "change": change})
        }
        R::NoSuchChange(change) => object!({"change": change}),
        R::ChangeAhead { change, updated } => object!({
            "change": change,
            "updated": updated.as_str(),
        }),
        R::DiscussionClosed(id) => object!({"discussion": id.as_str()}),
        R::DiscussionLinked { discussion, task } => object!({
            "discussion": discussion.as_str(),
            "task": task.as_str(),
        }),
        R::DiscussionAhead {
            discussion,
            updated,
        } => object!({
            "discussion": discussion.as_str(),
            "updated": updated.as_str(),
        }),
        R::MessageAhead { discussion, at } => object!({
            "discussion": discussion.as_str(),
            "at": at.as_str(),
        }),
        R::DiscussionFull { discussion, limit } => object!({
            "discussion": discussion.as_str(),
            "limit": limit.as_str(),
        }),
        R::ReviewAhead { change, updated } => object!({
            "change": change,
            "updated": updated.as_str(),
        }),
        R::ChangesLinked(changes) => object!({"changes": changes}),
        R::PrUnread(pr) => object!({"pr": pr}),
        R::PrNotOpen(pr) => object!({"pr": pr}),
        R::PrHeadMismatch(pr) => object!({"pr": pr}),
        R::ChangeUnlinked(change) => object!({"change": change}),
        R::ChangeMerged(change) => object!({"change": change}),
        R::NoChanges => Map::new(),
        R::ChangesUnmerged(changes) => object!({"changes": changes}),
        R::ChangesInconsistent(why) => {
            object!({"reason": shown(why)})
        }
        R::NotTakeable(status) => object!({"status": status.as_str()}),
        R::AlreadyHolder => Map::new(),
        R::NoPrs => Map::new(),
        R::PrsUnmerged(prs) => object!({"prs": prs}),
        R::PrsLinked(prs) => object!({"prs": prs}),
        R::PrNotLinked(pr) => object!({"pr": pr}),
        R::PrAlreadyLinked(pr) => object!({"pr": pr}),
        R::NotDraft(status) => object!({"status": status.as_str()}),
        R::NoEdit => Map::new(),
        R::Superseded { by } => {
            let by: Vec<&str> = by.iter().map(Slug::as_str).collect();
            object!({"by": by})
        }
        R::IssueShared { issue, tasks } => object!({
            "issue": issue,
            "tasks": [tasks[0].as_str(), tasks[1].as_str()],
        }),
        R::IssueIsPr {
            issue,
            task,
            pr_holder,
        } => object!({
            "issue": issue,
            "task": task.as_str(),
            "pr_holder": pr_holder.as_str(),
        }),
        R::OwnPrReferenced { task, pr, change } => object!({
            "task": task.as_str(),
            "pr": pr,
            "change": change,
        }),
        R::Invalid(schema) => object!({
            "field": shown(&schema.field),
            "problem": shown(&schema.problem),
        }),
    }
}

#[cfg(test)]
#[path = "answer_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "answer_show_tests.rs"]
mod show_tests;
