//! A task: the overall job a project plans, readies and claims. A [`Task`]
//! is only ever built from fields that pass [`Task::new`], so an invalid
//! task cannot exist; change one by taking its fields and building anew.
use super::super::value_objects::schema_error::{SchemaError, distinct, line, markdown};
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::{TaskKind, TaskStatus};
use super::claim::Claim;
use super::task_parts::{
    Item, MAX_ITEMS, MAX_REVIEWS, MAX_RUNS, MAX_STEPS, PlanStep, Review, Run, Team,
};
use std::collections::BTreeMap;

/// GitHub's issue title limit.
pub const MAX_TITLE_CHARS: usize = 256;
/// GitHub's issue body limit.
pub const MAX_DESCRIPTION_CHARS: usize = 65_536;
pub const MAX_DEPENDENCIES: usize = 64;
pub const MAX_PRS: usize = 16;

/// A task's content, unchecked: what [`Task::new`] validates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskFields {
    pub id: Slug,
    pub title: String,
    pub kind: TaskKind,
    /// Markdown.
    pub description: String,
    pub status: TaskStatus,
    /// Children are not stored: they are the tasks naming this one.
    pub parent: Option<Slug>,
    pub depends_on: Vec<Slug>,
    pub plan: Vec<PlanStep>,
    pub items: Vec<Item>,
    pub team: Option<Team>,
    pub claim: Option<Claim>,
    pub runs: Vec<Run>,
    pub reviews: Vec<Review>,
    /// Every pull request delivering the task, each numbered from 1.
    pub prs: Vec<u64>,
    pub created: Timestamp,
    pub updated: Timestamp,
}

/// A task whose fields keep every rule of the schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task(TaskFields);

impl Task {
    pub fn new(fields: TaskFields) -> Result<Self, SchemaError> {
        validate(&fields)?;
        Ok(Self(fields))
    }

    pub fn fields(&self) -> &TaskFields {
        &self.0
    }

    pub fn into_fields(self) -> TaskFields {
        self.0
    }
}

impl TryFrom<TaskFields> for Task {
    type Error = SchemaError;
    fn try_from(fields: TaskFields) -> Result<Self, SchemaError> {
        Self::new(fields)
    }
}

fn validate(task: &TaskFields) -> Result<(), SchemaError> {
    line("title", &task.title, MAX_TITLE_CHARS)?;
    markdown("description", &task.description, MAX_DESCRIPTION_CHARS)?;
    if task.parent.as_ref() == Some(&task.id) {
        return Err(SchemaError::new("parent", "a task is not its own parent"));
    }
    distinct("depends_on", &task.depends_on, MAX_DEPENDENCIES)?;
    if let Some(index) = task
        .depends_on
        .iter()
        .position(|dependency| *dependency == task.id)
    {
        return Err(SchemaError::new(
            format!("depends_on/{index}"),
            "a task does not depend on itself",
        ));
    }
    validate_plan_and_items(task)?;
    if let Some(team) = &task.team {
        team.validate("team")?;
    }
    validate_claim(task)?;
    validate_runs(task)?;
    let ids: Vec<&Slug> = task.reviews.iter().map(|review| &review.id).collect();
    distinct("reviews", &ids, MAX_REVIEWS)?;
    for (index, review) in task.reviews.iter().enumerate() {
        review.validate(&format!("reviews/{index}"))?;
    }
    distinct("prs", &task.prs, MAX_PRS)?;
    if let Some(index) = task.prs.iter().position(|number| !matches!(number, 1..)) {
        return Err(SchemaError::new(
            format!("prs/{index}"),
            "a pull request number starts at 1",
        ));
    }
    if task.created <= task.updated {
        Ok(())
    } else {
        Err(SchemaError::new("updated", "must not be before created"))
    }
}

fn validate_plan_and_items(task: &TaskFields) -> Result<(), SchemaError> {
    if task.plan.len() > MAX_STEPS {
        return Err(SchemaError::new(
            "plan",
            format!("must hold at most {MAX_STEPS} steps"),
        ));
    }
    for (index, step) in task.plan.iter().enumerate() {
        step.validate(&format!("plan/{index}"))?;
    }
    let ids: Vec<&Slug> = task.items.iter().map(|item| &item.id).collect();
    distinct("items", &ids, MAX_ITEMS)?;
    for (index, item) in task.items.iter().enumerate() {
        item.validate(&format!("items/{index}"))?;
        let siblings = item
            .depends_on
            .iter()
            .all(|dependency| *dependency != item.id && ids.contains(&dependency));
        if !siblings {
            return Err(SchemaError::new(
                format!("items/{index}/depends_on"),
                "must name other items of this task",
            ));
        }
    }
    match blocked_by_cycle(&task.items) {
        Some(index) => Err(SchemaError::new(
            format!("items/{index}/depends_on"),
            "is on or behind a dependency cycle",
        )),
        None => Ok(()),
    }
}

/// The first item that can never start because its dependencies loop:
/// items are settled once all they depend on are, until none settles.
/// Every dependency names a sibling (checked before).
fn blocked_by_cycle(items: &[Item]) -> Option<usize> {
    let position: BTreeMap<&Slug, usize> = items
        .iter()
        .enumerate()
        .map(|(index, item)| (&item.id, index))
        .collect();
    let mut settled = vec![false; items.len()];
    loop {
        let ready: Vec<usize> = (0..items.len())
            .filter(|&index| !settled[index])
            .filter(|&index| {
                items[index]
                    .depends_on
                    .iter()
                    .all(|dependency| settled[position[dependency]])
            })
            .collect();
        if ready.is_empty() {
            return settled.iter().position(|done| !done);
        }
        ready.into_iter().for_each(|index| settled[index] = true);
    }
}

/// Distinct ids; at most one open run, and only while the task is held.
fn validate_runs(task: &TaskFields) -> Result<(), SchemaError> {
    let ids: Vec<&Slug> = task.runs.iter().map(|run| &run.id).collect();
    distinct("runs", &ids, MAX_RUNS)?;
    for (index, run) in task.runs.iter().enumerate() {
        run.validate(&format!("runs/{index}"))?;
    }
    let open = task.runs.iter().filter(|run| run.is_open()).count();
    if open == 0 || (open == 1 && task.status.requires_claim()) {
        Ok(())
    } else {
        Err(SchemaError::new(
            "runs",
            "at most one run is open, and only while the task is claimed or in progress",
        ))
    }
}

/// Claimed and in-progress tasks have a holder; review and blocked may
/// keep one; any other status has none. A claim starts no earlier than
/// the task.
fn validate_claim(task: &TaskFields) -> Result<(), SchemaError> {
    match &task.claim {
        Some(claim) if task.status.may_hold_claim() => {
            claim.validate("claim")?;
            if claim.since >= task.created {
                Ok(())
            } else {
                Err(SchemaError::new(
                    "claim/since",
                    "must not be before the task was created",
                ))
            }
        }
        None if !task.status.requires_claim() => Ok(()),
        _ => Err(SchemaError::new(
            "claim",
            format!("does not match status {}", task.status.as_str()),
        )),
    }
}

#[cfg(test)]
#[path = "task_tests.rs"]
mod tests;
