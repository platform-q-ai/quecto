//! A task: the overall job a project plans, readies and claims. A [`Task`]
//! is only ever built from fields that pass [`Task::new`], so an invalid
//! task cannot exist; change one by taking its fields and building anew.
use super::super::value_objects::schema_error::{SchemaError, distinct, line, markdown};
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::{TaskKind, TaskStatus};
use super::claim::Claim;

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
    pub claim: Option<Claim>,
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
    validate_claim(task)?;
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
