//! How an answer names a board's artifacts (#2481 M4a): a file set aside
//! by its kind's key, `show`'s task with its plans and runs, in the
//! board's own field names as their files hold them, and nothing more;
//! and a listed task's summary of them.
use serde_json::{Map, Value, json};

use super::{
    error_code, fields, head_json, identity_json, key_fields, quarantined_list, shown, shown_json,
    slug_json, slugs_json, task_json,
};
use crate::application::project_board::dto::ShownTask;
use crate::application::project_board::operations::PlanPage;
use crate::application::project_board::ports::{ArtifactKey, QuarantinedFile};
use crate::application::project_board::task_summary::{Listed, RowSummaries, TaskSummary};
use crate::domain::project_board::entities::plan::{Plan, PlanItem};
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::team::Team;
use crate::domain::project_board::value_objects::shown_text::{
    shown_artifact_path, shown_board_path, shown_path,
};
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{PlanState, RunOutcome};

/// What `show` answers (#2481 M4a), all as one read of the board found
/// it: its `head`; the whole `task`; `plan`, the version `version` names
/// whole (#2481 M4b), or with none named the task's accepted plan
/// (`null` while it has none, or while its file is set aside);
/// `plans`, each version in brief, by version; `runs`, by number, each in
/// brief but the latest readable one, whole; `may_be_incomplete`,
/// whether a file that may hold one of its plans or runs is set aside, and
/// `quarantined`, those files, as a listing shows them.
pub(super) fn shown_fields(shown: &ShownTask, version: Option<u32>) -> Map<String, Value> {
    let latest = shown.latest_run().map(|run| run.fields().number);
    let runs = shown
        .runs
        .iter()
        .map(|run| match Some(run.fields().number) == latest {
            true => run_json(run),
            false => run_brief_json(run),
        });
    let plan = shown.plan(version);
    Map::from_iter([
        ("head".to_string(), head_json(&shown.head)),
        ("task".to_string(), task_json(&shown.task)),
        ("plan".to_string(), plan.map_or(Value::Null, plan_json)),
        (
            "plans".to_string(),
            Value::Array(shown.plans.iter().map(plan_brief_json).collect()),
        ),
        ("runs".to_string(), Value::Array(runs.collect())),
        (
            "may_be_incomplete".to_string(),
            Value::Bool(shown.may_be_incomplete()),
        ),
        (
            "quarantined".to_string(),
            quarantined_list(&shown.set_aside),
        ),
    ])
}

/// A listing's `head`, its `tasks` and the files set aside as
/// `quarantined`; asked for summaries, each task with its `summary`
/// ([`summary_json`]), or, when they could not be read, none and
/// `summary_skipped`: the code of why.
pub(super) fn listing_fields(listing: &Listed) -> Map<String, Value> {
    let tasks = listing.entries().map(|(task, summary)| {
        let mut entry = task_json(task);
        if let Some(summary) = summary {
            entry["summary"] = summary_json(summary);
        }
        entry
    });
    let mut fields = Map::from_iter([
        ("head".to_string(), head_json(listing.head())),
        ("tasks".to_string(), Value::Array(tasks.collect())),
        (
            "quarantined".to_string(),
            quarantined_list(listing.quarantined()),
        ),
    ]);
    match listing.summaries() {
        RowSummaries::Omitted | RowSummaries::Given(_) => {}
        RowSummaries::Skipped(why) => {
            let code = error_code(why).as_str();
            fields.insert("summary_skipped".into(), json!(code));
        }
    }
    fields
}

/// A plan in the board's own field names, as its file holds them, every
/// field present (`null` or `[]` when unset), in a fixed order: who
/// accepted it and when as `accepted_by` and `accepted_at`.
pub(super) fn plan_json(plan: &Plan) -> Value {
    let plan = plan.fields();
    let items: Vec<Value> = plan.items.iter().map(item_json).collect();
    json!({
        "task": slug_json(&plan.task),
        "version": plan.version,
        "state": plan.state.as_str(),
        "approach": plan.approach,
        "items": items,
        "team": plan.team.as_ref().map(team_json),
        "proposed_by": identity_json(&plan.proposed_by),
        "edited_by": plan.edited_by.as_ref().map(identity_json),
        "accepted_by": plan.accepted.as_ref().map(|(by, _)| identity_json(by)),
        "accepted_at": plan.accepted.as_ref().map(|(_, at)| at.as_str()),
        "created": plan.created.as_str(),
        "updated": plan.updated.as_str(),
    })
}

fn item_json(item: &PlanItem) -> Value {
    json!({
        "id": slug_json(&item.id),
        "title": item.title,
        "criteria": item.criteria,
        "depends_on": slugs_json(&item.depends_on),
    })
}

/// What a `show` given a page answers (#2481 M4a review round 2): its
/// `head`; the task's id as `task`; `plan`, the `version` of the plan
/// `version` names, or of the accepted plan, and its `updated` (#2481
/// M4b review round 2: a reader sees an edit land between its pages),
/// with only the parts the page names: `approach_from` and the
/// `approach` from that character, `items_from` and the task `items` from
/// that index (`null` while the task has no plan to page through); and
/// `may_be_incomplete`, as `show` says it.
pub(super) fn page_fields(
    shown: &ShownTask,
    page: PlanPage,
    version: Option<u32>,
) -> Map<String, Value> {
    let plan = shown.plan(version).map(|plan| {
        let plan = plan.fields();
        let mut part = fields([
            ("version", json!(plan.version)),
            ("updated", json!(plan.updated.as_str())),
        ]);
        if let Some(from) = page.approach_from() {
            let approach: String = plan.approach.chars().skip(from).collect();
            part.insert("approach_from".into(), json!(from));
            part.insert("approach".into(), json!(approach));
        }
        if let Some(from) = page.items_from() {
            let items: Vec<Value> = plan.items.iter().skip(from).map(item_json).collect();
            part.insert("items_from".into(), json!(from));
            part.insert("items".into(), Value::Array(items));
        }
        Value::Object(part)
    });
    Map::from_iter([
        ("head".to_string(), head_json(&shown.head)),
        ("task".to_string(), slug_json(&shown.task.fields().id)),
        ("plan".to_string(), plan.unwrap_or(Value::Null)),
        (
            "may_be_incomplete".to_string(),
            Value::Bool(shown.may_be_incomplete()),
        ),
    ])
}

fn team_json(team: &Team) -> Value {
    let roles: Vec<Value> = team
        .roles
        .iter()
        .map(|role| {
            json!({
                "name": slug_json(&role.name),
                "model": role.model,
                "effort": role.effort.as_str(),
                "members": role.limits.members,
                "turns": role.limits.turns,
            })
        })
        .collect();
    json!({
        "roles": roles,
        "deadline_seconds": team.deadline_seconds,
        "image": team.image,
    })
}

/// A plan version as `propose_plan` and `edit_plan` answer it (#2481
/// M4b): which it is and where it stands, without the text the caller
/// sent: `task`, `version`, `state`, `proposed_by`, `edited_by`,
/// `created` and `updated` (what an `edit_plan` passes back as
/// `expected_updated`).
pub(super) fn planned_json(plan: &Plan) -> Value {
    let plan = plan.fields();
    json!({
        "task": slug_json(&plan.task),
        "version": plan.version,
        "state": plan.state.as_str(),
        "proposed_by": identity_json(&plan.proposed_by),
        "edited_by": plan.edited_by.as_ref().map(identity_json),
        "created": plan.created.as_str(),
        "updated": plan.updated.as_str(),
    })
}

/// A plan version in brief: no text of it but who proposed it.
fn plan_brief_json(plan: &Plan) -> Value {
    let plan = plan.fields();
    json!({
        "version": plan.version,
        "state": plan.state.as_str(),
        "proposed_by": identity_json(&plan.proposed_by),
        "updated": plan.updated.as_str(),
    })
}

/// A run in the board's own field names, as its file holds them, every
/// field present, in a fixed order: `state` is `running` or its outcome,
/// as the file spells it, and its cost `cost_tokens`. Its items' owners
/// and evidence came from a swarm's report, so each is shown as text from
/// outside is ([`shown`]): credentials redacted.
pub(super) fn run_json(run: &Run) -> Value {
    let items: Vec<Value> = run
        .fields()
        .items
        .iter()
        .map(|item| {
            let evidence: Vec<String> = item.evidence.iter().map(|line| shown(line)).collect();
            json!({
                "id": slug_json(&item.id),
                "state": item.state.as_str(),
                "owner": item.owner.as_deref().map(shown),
                "evidence": evidence,
            })
        })
        .collect();
    let mut answer = Map::new();
    answer.insert("task".into(), slug_json(&run.fields().task));
    answer.extend(run_brief(run));
    answer.insert("items".into(), Value::Array(items));
    Value::Object(answer)
}

/// A run in brief: where it stands, never its items.
fn run_brief_json(run: &Run) -> Value {
    Value::Object(run_brief(run))
}

fn run_brief(run: &Run) -> Map<String, Value> {
    let run = run.fields();
    fields([
        ("number", json!(run.number)),
        ("plan_version", json!(run.plan_version)),
        ("state", json!(run_state(run.outcome))),
        ("started", json!(run.started.as_str())),
        (
            "ended",
            json!(run.ended.as_ref().map(|ended| ended.as_str())),
        ),
        ("cost_tokens", json!(run.cost)),
    ])
}

/// A run's `state`: `running` exactly while it has no outcome, its
/// outcome as its file spells it otherwise; `show` and a listing's
/// summary say it alike.
fn run_state(outcome: Option<RunOutcome>) -> &'static str {
    outcome.map_or("running", RunOutcome::as_str)
}

/// A listed task's `summary` (`TaskSummary`): its current `plan`'s
/// `version` and `state` (`draft` or `accepted`), its latest readable
/// `run` (`number`, `plan_version`, `state`, `started` and `ended` as
/// `show` gives them, its `deadline`, and its task `items` counted by
/// state), each `null` when it has none, and `may_be_incomplete`, as
/// `show` says it.
fn summary_json(summary: &TaskSummary) -> Value {
    let plan = summary.plan.as_ref().map(|plan| {
        assert!(
            matches!(plan.state, PlanState::Draft | PlanState::Accepted),
            "a summary names a draft or accepted plan: {plan:?}"
        );
        json!({"version": plan.version, "state": plan.state.as_str()})
    });
    let run = summary.run.as_ref().map(|run| {
        let items = &run.items;
        json!({
            "number": run.number,
            "plan_version": run.plan_version,
            "state": run_state(run.outcome),
            "started": run.started.as_str(),
            "ended": run.ended.as_ref().map(Timestamp::as_str),
            "deadline": run.deadline.as_ref().map(Timestamp::as_str),
            "items": {
                "total": items.total,
                "todo": items.todo,
                "doing": items.doing,
                "review": items.review,
                "done": items.done,
                "blocked": items.blocked,
            },
        })
    });
    json!({
        "plan": plan,
        "run": run,
        "may_be_incomplete": summary.may_be_incomplete,
    })
}

/// A file set aside, as a listing names it: by its raw `path`, its task's
/// `id` (`null` but for a task's file), another kind's key, and why.
pub(super) fn quarantined_json(file: &QuarantinedFile) -> Value {
    // A task's file in its own folder, another kind's in its kind's; a
    // stray in any folder read.
    let sanitised = match &file.key {
        Some(key) => match artifact_folder(key) {
            Some(folder) => shown_artifact_path(folder, &file.path),
            None => shown_path(&file.path),
        },
        None => shown_board_path(&file.path),
    };
    assert!(
        sanitised,
        "a store shows every path it sets aside sanitised: {:?}",
        file.path
    );
    let mut entry = Map::new();
    // Raw: the store allowlists or quotes every name it sets aside, a path
    // is no secret, and a clear names it exactly as shown.
    entry.insert("path".into(), json!(file.path));
    entry.insert("id".into(), json!(file.id().map(slug_json)));
    // Another kind's file by its key, as `artifact_set_aside` names it; a
    // task's by its `id`, as ever.
    if let Some(key) = file
        .key
        .as_ref()
        .filter(|key| artifact_folder(key).is_some())
    {
        entry.extend(key_fields(key));
    }
    entry.insert("reason".into(), shown_json(&file.reason));
    entry.insert("object".into(), json!(file.object));
    Value::Object(entry)
}

/// The folder the file of `key` lies in, for an artifact other than a
/// task (whose folder, `tasks/`, [`shown_path`] knows).
pub(super) fn artifact_folder(key: &ArtifactKey) -> Option<&'static str> {
    match key {
        ArtifactKey::Task(_) => None,
        ArtifactKey::Plan(..) => Some("plans"),
        ArtifactKey::Run(..) => Some("runs"),
        ArtifactKey::Change(..) => Some("changes"),
        ArtifactKey::Review(..) => Some("reviews"),
        ArtifactKey::Discussion(_) => Some("discussions"),
    }
}
