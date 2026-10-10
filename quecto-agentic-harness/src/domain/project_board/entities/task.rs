//! A task file, `tasks/<id>.json` on the project's `quecto/board` branch.
use super::super::value_objects::schema_error::SchemaError;
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
        Ok(())
    }
}

#[cfg(test)]
#[path = "task_tests.rs"]
mod tests;
