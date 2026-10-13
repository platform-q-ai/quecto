//! How an answer names a board's artifacts (#2481 M4a): a file set aside
//! by its kind's key, and `show`'s task with its plans and runs, in the
//! board's own field names as their files hold them, and nothing more.
use serde_json::{Map, Value, json};

use super::{
    fields, head_json, identity_json, key_fields, quarantined_list, shown, shown_json, slug_json,
    slugs_json, task_json,
};
use crate::application::project_board::dto::ShownTask;
use crate::application::project_board::operations::PlanPage;
use crate::application::project_board::ports::{ArtifactKey, QuarantinedFile};
use crate::domain::project_board::entities::plan::{Plan, PlanItem};
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::team::Team;
use crate::domain::project_board::value_objects::shown_text::{
    shown_artifact_path, shown_board_path, shown_path,
};

/// What `show` answers (#2481 M4a), all as one read of the board found
/// it: its `head`; the whole `task`; `plan`, the task's accepted plan
/// whole (`null` while it has none, or while its file is set aside);
/// `plans`, each version in brief, by version; `runs`, by number, each in
/// brief but the latest readable one, whole; `may_be_incomplete`,
/// whether a file that may hold one of its plans or runs is set aside, and
/// `quarantined`, those files, as a listing shows them.
pub(super) fn shown_fields(shown: &ShownTask) -> Map<String, Value> {
    let latest = shown.latest_run().map(|run| run.fields().number);
    let runs = shown
        .runs
        .iter()
        .map(|run| match Some(run.fields().number) == latest {
            true => run_json(run),
            false => run_brief_json(run),
        });
    let plan = shown.accepted_plan();
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

/// A plan in the board's own field names, as its file holds them, every
/// field present (`null` or `[]` when unset), in a fixed order: who
/// accepted it and when as `accepted_by` and `accepted_at`, and its team's
/// budget as `budget_tokens`.
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
/// `head`; the task's id as `task`; `plan`, the accepted plan's `version`
/// with only the parts the page names: `approach_from` and the
/// `approach` from that character, `items_from` and the task `items` from
/// that index (`null` while the task has no plan to page through); and
/// `may_be_incomplete`, as `show` says it.
pub(super) fn page_fields(shown: &ShownTask, page: PlanPage) -> Map<String, Value> {
    let plan = shown.accepted_plan().map(|plan| {
        let plan = plan.fields();
        let mut part = fields([("version", json!(plan.version))]);
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
        "budget_tokens": team.budget.as_ref().map(|budget| budget.tokens),
        "deadline": team.deadline.as_ref().map(|deadline| deadline.as_str()),
        "image": team.image,
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
    // `running` exactly while it has no outcome, as its file spells it.
    let state = run.outcome.map_or("running", |outcome| outcome.as_str());
    fields([
        ("number", json!(run.number)),
        ("plan_version", json!(run.plan_version)),
        ("state", json!(state)),
        ("started", json!(run.started.as_str())),
        (
            "ended",
            json!(run.ended.as_ref().map(|ended| ended.as_str())),
        ),
        ("cost_tokens", json!(run.cost)),
    ])
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
