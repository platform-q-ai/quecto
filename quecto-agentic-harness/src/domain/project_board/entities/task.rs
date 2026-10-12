//! A task: the overall job a project plans, readies and claims. A [`Task`]
//! is only ever built from fields that pass [`Task::new`], so an invalid
//! task cannot exist; change one by taking its fields and building anew.
use super::super::value_objects::schema_error::{SchemaError, distinct, line, markdown};
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::{TaskKind, TaskStatus};
use super::claim::Claim;
use super::plan::MAX_PLAN_VERSION;

/// GitHub's issue title limit.
pub const MAX_TITLE_CHARS: usize = 256;
/// GitHub's issue body limit.
pub const MAX_DESCRIPTION_CHARS: usize = 65_536;
pub const MAX_DEPENDENCIES: usize = 64;
pub const MAX_PRS: usize = 16;
/// All of a task's text together, in UTF-8 bytes: this budget keeps every
/// task the domain admits inside the file it is stored in (the record
/// layer proves it). Its title and description alone always fit (a
/// character is at most four bytes); the budget bounds them with the rest
/// of a task's text as it gains more.
pub const MAX_TEXT_BYTES: usize = 320 * 1024;
const _: () = assert!(4 * (MAX_TITLE_CHARS + MAX_DESCRIPTION_CHARS) <= MAX_TEXT_BYTES);

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
    /// The version of the task's plan it was accepted at; the plan (its
    /// items and team) and the runs of it are artifacts of their own.
    pub accepted_plan: Option<u32>,
    pub claim: Option<Claim>,
    /// Every pull request delivering the task, each numbered from 1.
    pub prs: Vec<u64>,
    pub created: Timestamp,
    pub updated: Timestamp,
}

impl TaskFields {
    /// The UTF-8 bytes of every free text in the task: its title and
    /// description. Ids, words, numbers, times and the claim's holder are
    /// not text: their size is bounded by the field limits, so taking a
    /// claim never runs a task over its budget. Review findings are
    /// review artifacts of their own (M1c), with their own budget.
    pub fn text_bytes(&self) -> usize {
        self.title.len() + self.description.len()
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
    match task
        .accepted_plan
        .is_none_or(|version| (1..=MAX_PLAN_VERSION).contains(&version))
    {
        true => {}
        false => {
            return Err(SchemaError::new(
                "accepted_plan",
                format!("must be a plan version from 1 to {MAX_PLAN_VERSION}"),
            ));
        }
    }
    validate_claim(task)?;
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
