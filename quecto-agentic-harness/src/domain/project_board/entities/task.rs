//! A task: the overall job a project plans, readies and claims. A [`Task`]
//! is only ever built from fields that pass [`Task::new`], so an invalid
//! task cannot exist; change one by taking its fields and building anew.
use super::super::value_objects::schema_error::{SchemaError, distinct, line, markdown};
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::{TaskKind, TaskStatus};
use super::claim::{Claim, Identity};
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
/// All of a task's text together, in UTF-8 bytes. The field limits alone
/// would add up to megabytes; this budget keeps every task the domain
/// admits inside the file it is stored in (the record layer proves it).
pub const MAX_TEXT_BYTES: usize = 320 * 1024;

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

impl TaskFields {
    /// The UTF-8 bytes of every free text in the task: titles, markdown,
    /// criteria, evidence, owners, models, the image, reviewers, review
    /// places, findings and tests. Ids, words, numbers, times and the
    /// claim's holder are not text: their size is bounded by the field
    /// limits, so taking a claim never runs a task over its budget.
    pub fn text_bytes(&self) -> usize {
        let sum = |texts: &[String]| texts.iter().map(String::len).sum::<usize>();
        let optional = |text: &Option<String>| text.as_ref().map_or(0, String::len);
        let identity = |who: &Identity| who.name.len() + who.email.len();
        let plan = self
            .plan
            .iter()
            .map(|step| step.title.len() + sum(&step.criteria));
        let items = self.items.iter().map(|item| {
            item.title.len() + sum(&item.criteria) + optional(&item.owner) + sum(&item.evidence)
        });
        let team = self.team.iter().map(|team| {
            let models: usize = team.roles.iter().map(|role| role.model.len()).sum();
            models + optional(&team.image)
        });
        let reviews = self.reviews.iter().map(|review| {
            identity(&review.reviewer)
                + optional(&review.location)
                + review.finding.len()
                + optional(&review.proving_test)
        });
        let parts: usize = plan.chain(items).chain(team).chain(reviews).sum();
        self.title.len() + self.description.len() + parts
    }
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
    // Other times are ordered against these two, so they come first.
    if task.created > task.updated {
        return Err(SchemaError::new("updated", "must not be before created"));
    }
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
    validate_times(task)?;
    distinct("prs", &task.prs, MAX_PRS)?;
    if let Some(index) = task.prs.iter().position(|number| !matches!(number, 1..)) {
        return Err(SchemaError::new(
            format!("prs/{index}"),
            "a pull request number starts at 1",
        ));
    }
    let text = task.text_bytes();
    if text <= MAX_TEXT_BYTES {
        Ok(())
    } else {
        Err(SchemaError::new(
            "text",
            format!("the task's text is {text} bytes; the most is {MAX_TEXT_BYTES}"),
        ))
    }
}

/// Runs and reviews happen while the task exists, by its last change:
/// from `created` to `updated`, like a claim's `since`. A deadline falls
/// after the task was created.
fn validate_times(task: &TaskFields) -> Result<(), SchemaError> {
    let during = |time: &Timestamp| task.created <= *time && *time <= task.updated;
    let mut times = task.runs.iter().enumerate().flat_map(|(index, run)| {
        let ended = run
            .ended
            .iter()
            .map(move |ended| (format!("runs/{index}/ended"), ended));
        std::iter::once((format!("runs/{index}/started"), &run.started)).chain(ended)
    });
    let mut reviews = task
        .reviews
        .iter()
        .enumerate()
        .map(|(index, review)| (format!("reviews/{index}/at"), &review.at));
    if let Some((field, _)) = times
        .find(|(_, time)| !during(time))
        .or_else(|| reviews.find(|(_, time)| !during(time)))
    {
        return Err(SchemaError::new(
            field,
            "must be from when the task was created to when it was updated",
        ));
    }
    let deadline = task.team.as_ref().and_then(|team| team.deadline.as_ref());
    if deadline.is_none_or(|deadline| *deadline > task.created) {
        Ok(())
    } else {
        Err(SchemaError::new(
            "team/deadline",
            "must be after the task was created",
        ))
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

/// Distinct ids; at most one open run, and only while the task holds a
/// claim (it is claimed or in progress, or in review or blocked with its
/// claim kept): someone answers for a run that is going.
fn validate_runs(task: &TaskFields) -> Result<(), SchemaError> {
    let ids: Vec<&Slug> = task.runs.iter().map(|run| &run.id).collect();
    distinct("runs", &ids, MAX_RUNS)?;
    for (index, run) in task.runs.iter().enumerate() {
        run.validate(&format!("runs/{index}"))?;
    }
    let open = task.runs.iter().filter(|run| run.is_open()).count();
    let held = task.status.may_hold_claim() && task.claim.is_some();
    if open == 0 || (open == 1 && held) {
        Ok(())
    } else {
        Err(SchemaError::new(
            "runs",
            "at most one run is open, and only while the task holds a claim",
        ))
    }
}

/// Claimed and in-progress tasks have a holder; review and blocked may
/// keep one; any other status has none. A claim starts while the task
/// exists: no earlier than `created` and no later than `updated`, the
/// change that took it.
fn validate_claim(task: &TaskFields) -> Result<(), SchemaError> {
    match &task.claim {
        Some(claim) if task.status.may_hold_claim() => {
            claim.validate("claim")?;
            if task.created <= claim.since && claim.since <= task.updated {
                Ok(())
            } else {
                Err(SchemaError::new(
                    "claim/since",
                    "must be from when the task was created to when it was updated",
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
