//! A board operation's answer as its front ends send it: one JSON object
//! of a stable shape.
//!
//! - Success: `{"ok": true, "op": …}` plus what the op answers: `task` (and
//!   after a change that may complete it, `completion`), a listing's
//!   `head`, `tasks` and `quarantined`, or `head` alone; and `clock` when a
//!   change was not bounded against an outside time.
//! - Refusal: `{"ok": false, "op": …, "error": {"code", "field",
//!   "message", …}}`. `field` is the request field at fault, `null` when
//!   the refusal is not about one. Each [`ErrorCode`] has one shape: the
//!   same fields every time.
//!   Messages are the error's own words (its `Display`), with any URL's
//!   credentials redacted, at most [`MAX_MESSAGE_CHARS`] characters.
//!
//! A task's shape is the board's own field names, every field always
//! present (`null` or `[]` when unset), in a fixed order.
use serde_json::{Map, Value, json};

use super::request::RequestError;
use crate::application::project_board::dto::{BoardListing, ProjectBoardError};
use crate::application::project_board::operations::BoardOutcome;
use crate::application::project_board::ports::{
    BoardClockError, BoardHead, BoardStoreError, PullRequestStatesError, QuarantinedFile,
};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::redaction::redact_url_userinfo;

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
    /// `read`.
    ClockUnreadable,
    /// `task`.
    SetAside,
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
    /// `path`.
    NotSetAside,
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
            Self::Rule => "rule",
            Self::InvalidTask => "invalid_task",
            Self::InvalidWriter => "invalid_writer",
            Self::NotFound => "not_found",
            Self::Exists => "exists",
            Self::ClockUnreadable => "clock_unreadable",
            Self::SetAside => "set_aside",
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
        }
    }
}

/// Text an answer may carry: credentials in URLs redacted, bounded.
pub fn shown(text: &str) -> String {
    let redacted = redact_url_userinfo(text);
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
        BoardOutcome::Shown { head, task } => {
            fields.insert("head".into(), head_json(head));
            fields.insert("task".into(), task_json(task));
        }
        BoardOutcome::Changed { task, completion } => {
            fields.insert("task".into(), task_json(task));
            if let Some(completion) = completion {
                let completion = match completion.as_ref() {
                    Ok(task) => json!({"done": true, "task": task_json(task)}),
                    Err(error) => json!({"done": false, "error": error_json(error)}),
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
            Value::Array(listing.quarantined.iter().map(quarantined_json).collect()),
        ),
    ])
}

fn head_json(head: &BoardHead) -> Value {
    Value::String(head.0.clone())
}

fn slug_json(slug: &Slug) -> Value {
    Value::String(slug.as_str().into())
}

fn slugs_json(slugs: &[Slug]) -> Value {
    Value::Array(slugs.iter().map(slug_json).collect())
}

fn identity_json(identity: &Identity) -> Value {
    json!({"name": identity.name, "email": identity.email})
}

fn quarantined_json(file: &QuarantinedFile) -> Value {
    json!({
        "path": file.path,
        "id": file.id.as_ref().map(slug_json),
        "reason": shown(&file.reason),
    })
}

/// A task in the board's own field names, every field present.
pub fn task_json(task: &Task) -> Value {
    let task = task.fields();
    let items: Vec<Value> = task
        .items
        .iter()
        .map(|item| {
            json!({
                "id": slug_json(&item.id),
                "title": item.title,
                "criteria": item.criteria,
                "depends_on": slugs_json(&item.depends_on),
                "state": item.state.as_str(),
                "owner": item.owner,
                "evidence": item.evidence,
            })
        })
        .collect();
    let team = task.team.as_ref().map(|team| {
        let roles: Vec<Value> = team
            .roles
            .iter()
            .map(|role| {
                json!({
                    "name": slug_json(&role.name),
                    "model": role.model,
                    "effort": role.effort.as_str(),
                    "limits": {"members": role.limits.members, "turns": role.limits.turns},
                })
            })
            .collect();
        json!({
            "roles": roles,
            "budget": team.budget.as_ref().map(|budget| json!({"tokens": budget.tokens})),
            "deadline": team.deadline.as_ref().map(|at| at.as_str()),
            "image": team.image,
        })
    });
    let runs: Vec<Value> = task
        .runs
        .iter()
        .map(|run| {
            json!({
                "id": slug_json(&run.id),
                "started": run.started.as_str(),
                "ended": run.ended.as_ref().map(|at| at.as_str()),
                "outcome": run.outcome.map(|outcome| outcome.as_str()),
            })
        })
        .collect();
    let reviews: Vec<Value> = task
        .reviews
        .iter()
        .map(|review| {
            json!({
                "id": slug_json(&review.id),
                "round": review.round,
                "reviewer": identity_json(&review.reviewer),
                "at": review.at.as_str(),
                "severity": review.severity.as_str(),
                "location": review.location,
                "finding": review.finding,
                "verdict": review.verdict.as_str(),
                "proving_test": review.proving_test,
            })
        })
        .collect();
    let plan: Vec<Value> = task
        .plan
        .iter()
        .map(|step| json!({"title": step.title, "criteria": step.criteria}))
        .collect();
    json!({
        "id": slug_json(&task.id),
        "title": task.title,
        "kind": task.kind.as_str(),
        "description": task.description,
        "status": task.status.as_str(),
        "parent": task.parent.as_ref().map(slug_json),
        "depends_on": slugs_json(&task.depends_on),
        "plan": plan,
        "items": items,
        "team": team,
        "claim": task.claim.as_ref().map(|claim| json!({
            "holder": identity_json(&claim.holder),
            "since": claim.since.as_str(),
            "expires": claim.expires.as_str(),
        })),
        "runs": runs,
        "reviews": reviews,
        "prs": task.prs,
        "created": task.created.as_str(),
        "updated": task.updated.as_str(),
    })
}

/// A refused answer to `op` (`None` when the request named no op it
/// knows), with `code`'s own fields.
pub fn refused(
    op: Option<&str>,
    code: ErrorCode,
    message: &str,
    fields: Map<String, Value>,
) -> Value {
    let mut error = Map::new();
    error.insert("code".into(), Value::String(code.as_str().into()));
    error.insert("message".into(), shown_json(message));
    error.extend(fields);
    with_field(&mut error);
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

/// A front end's own refusal with a `reason` (the store could not be
/// built, or no committer is configured).
pub fn reason_refused(op: Option<&str>, code: ErrorCode, reason: &str) -> Value {
    refused(op, code, reason, fields([("reason", shown_json(reason))]))
}

/// An operation the board refused.
pub fn board_refused(op: &str, error: &ProjectBoardError) -> Value {
    json!({"ok": false, "op": op, "error": error_json(error)})
}

fn error_json(error: &ProjectBoardError) -> Value {
    let (code, fields) = error_fields(error);
    let mut object = Map::new();
    object.insert("code".into(), Value::String(code.as_str().into()));
    object.insert("message".into(), shown_json(&error.to_string()));
    object.extend(fields);
    with_field(&mut object);
    Value::Object(object)
}

/// Every refusal names a `field`: its own, or `null`.
fn with_field(error: &mut Map<String, Value>) {
    error.entry("field").or_insert(Value::Null);
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
        E::Rule(rule) => (C::Rule, fields([("rule", rule_json(rule))])),
        E::Invalid(schema) => (
            C::InvalidTask,
            fields([
                ("field", json!(schema.field)),
                ("problem", json!(schema.problem)),
            ]),
        ),
        E::InvalidWriter(schema) => (
            C::InvalidWriter,
            fields([
                ("field", json!(schema.field)),
                ("problem", json!(schema.problem)),
            ]),
        ),
        E::NotFound(id) => (C::NotFound, fields([("task", slug_json(id))])),
        E::Exists(id) => (C::Exists, fields([("task", slug_json(id))])),
        E::Clock(BoardClockError::OutOfRange(read)) => {
            (C::ClockUnreadable, fields([("read", shown_json(read))]))
        }
        E::Quarantined(id) => (C::SetAside, fields([("task", slug_json(id))])),
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
        E::NotSetAside(path) => (C::NotSetAside, fields([("path", shown_json(path))])),
    }
}

/// A board rule's refusal: `{"code": …}` and the rule's own fields.
fn rule_json(rule: &BoardRuleError) -> Value {
    use BoardRuleError as R;
    match rule {
        R::NotAllowed { from, to } => {
            json!({"code": "not_allowed", "from": from.as_str(), "to": to.as_str()})
        }
        R::NotHolder => json!({"code": "not_holder"}),
        R::NoClaim => json!({"code": "no_claim"}),
        R::ClaimLive => json!({"code": "claim_live"}),
        R::ClaimExpired => json!({"code": "claim_expired"}),
        R::RunOpen(run) => json!({"code": "run_open", "run": run.as_str()}),
        R::NoOpenRun => json!({"code": "no_open_run"}),
        R::NotTakeable(status) => json!({"code": "not_takeable", "status": status.as_str()}),
        R::AlreadyHolder => json!({"code": "already_holder"}),
        R::NoPrs => json!({"code": "no_prs"}),
        R::PrsUnmerged(prs) => json!({"code": "prs_unmerged", "prs": prs}),
        R::PrsLinked(prs) => json!({"code": "prs_linked", "prs": prs}),
        R::PrNotLinked(pr) => json!({"code": "pr_not_linked", "pr": pr}),
        R::PrAlreadyLinked(pr) => json!({"code": "pr_already_linked", "pr": pr}),
        R::NotDraft(status) => json!({"code": "not_draft", "status": status.as_str()}),
        R::NoEdit => json!({"code": "no_edit"}),
        R::Invalid(schema) => {
            json!({"code": "invalid", "field": schema.field, "problem": schema.problem})
        }
    }
}
