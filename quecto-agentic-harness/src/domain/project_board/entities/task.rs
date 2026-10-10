//! A task file, `tasks/<id>.json` on the project's `quecto/board` branch.
use super::super::value_objects::schema_error::{SchemaError, distinct, line, text};
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::{TaskKind, TaskStatus};
use super::claim::Claim;
use serde::{Deserialize, Serialize};

pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_DESCRIPTION_BYTES: usize = 64 * 1024;
/// The most children or dependencies.
pub const MAX_LIST: usize = 256;

/// The overall job. Field order is the file's key order: keep it stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: Slug,
    pub title: String,
    pub kind: TaskKind,
    pub description: String,
    pub status: TaskStatus,
    pub parent: Option<Slug>,
    pub children: Vec<Slug>,
    pub depends_on: Vec<Slug>,
    pub claim: Option<Claim>,
    pub pr: Option<u64>,
    pub created: Timestamp,
    pub updated: Timestamp,
}

impl Task {
    /// Every rule a task file must keep; checked on every read and write.
    pub fn validate(&self) -> Result<(), SchemaError> {
        line("title", &self.title, MAX_TITLE_CHARS)?;
        text("description", &self.description, MAX_DESCRIPTION_BYTES)?;
        distinct("children", &self.children, MAX_LIST)?;
        distinct("depends_on", &self.depends_on, MAX_LIST)?;
        if self.parent.as_ref() == Some(&self.id) {
            return Err(SchemaError::new("parent", "a task is not its own parent"));
        }
        if self.children.contains(&self.id) {
            return Err(SchemaError::new("children", "a task is not its own child"));
        }
        if self.depends_on.contains(&self.id) {
            return Err(SchemaError::new(
                "depends_on",
                "a task does not depend on itself",
            ));
        }
        self.validate_claim()?;
        if self.pr == Some(0) {
            return Err(SchemaError::new("pr", "a pull request number starts at 1"));
        }
        if self.created <= self.updated {
            Ok(())
        } else {
            Err(SchemaError::new("updated", "must not be before created"))
        }
    }

    /// Claimed and in-progress tasks have a holder; review and blocked may
    /// keep one; any other status has none.
    fn validate_claim(&self) -> Result<(), SchemaError> {
        match &self.claim {
            Some(claim) if self.status.may_hold_claim() => claim.validate(),
            None if !self.status.requires_claim() => Ok(()),
            _ => Err(SchemaError::new(
                "claim",
                format!("does not match status {:?}", self.status),
            )),
        }
    }
}

#[cfg(test)]
#[path = "task_tests.rs"]
mod tests;
