//! A change: a pointer to the code that delivers a task (#2481's artifact
//! model). It names a branch, the pull request while GitHub hosts pull
//! requests, and the merge commit once merged; commits, CI and merge state
//! are read live from git and GitHub, never copied. Its parent is the
//! task. An unlinked change (a wrong or abandoned one) is kept, marked, so
//! nothing is deleted.
use super::super::value_objects::code_ref::{BranchName, CommitId, MAX_BRANCH_BYTES};
use super::super::value_objects::schema_error::SchemaError;
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::claim::Identity;

/// The most changes a task links.
pub const MAX_CHANGE_NUMBER: u32 = 999;
/// A change's text (its branch name), budgeted like every artifact's.
pub const MAX_CHANGE_TEXT_BYTES: usize = MAX_BRANCH_BYTES;

/// A change's content, unchecked: what [`Change::new`] validates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeFields {
    /// The task it delivers.
    pub task: Slug,
    /// From 1, per task.
    pub number: u32,
    pub branch: BranchName,
    /// The pull request, from 1, while GitHub hosts pull requests.
    pub pr: Option<u64>,
    /// The merge commit, once merged: a branch may be deleted or
    /// rewritten, a commit id stays.
    pub merged: Option<CommitId>,
    pub linked_by: Identity,
    /// Who unlinked it and when: a wrong or abandoned change, kept.
    pub unlinked: Option<(Identity, Timestamp)>,
    pub created: Timestamp,
    pub updated: Timestamp,
}

impl ChangeFields {
    /// The UTF-8 bytes of the change's free text: its branch name.
    pub fn text_bytes(&self) -> usize {
        self.branch.as_str().len()
    }
}

/// A change whose fields keep every rule of its schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change(ChangeFields);

impl Change {
    pub fn new(fields: ChangeFields) -> Result<Self, SchemaError> {
        validate(&fields)?;
        Ok(Self(fields))
    }

    pub fn fields(&self) -> &ChangeFields {
        &self.0
    }

    pub fn into_fields(self) -> ChangeFields {
        self.0
    }

    /// Still delivering its task: linked, not unlinked.
    pub fn is_linked(&self) -> bool {
        self.0.unlinked.is_none()
    }
}

fn validate(change: &ChangeFields) -> Result<(), SchemaError> {
    match change.created <= change.updated {
        true => {}
        false => return Err(SchemaError::new("updated", "must not be before created")),
    }
    match (1..=MAX_CHANGE_NUMBER).contains(&change.number) {
        true => {}
        false => {
            return Err(SchemaError::new(
                "number",
                format!("must be 1 to {MAX_CHANGE_NUMBER}"),
            ));
        }
    }
    match change.pr.is_none_or(|pr| pr >= 1) {
        true => {}
        false => return Err(SchemaError::new("pr", "a pull request number starts at 1")),
    }
    change.linked_by.validate("linked_by")?;
    let text = change.text_bytes();
    match text <= MAX_CHANGE_TEXT_BYTES {
        true => {}
        false => {
            return Err(SchemaError::new(
                "text",
                format!("the change's text is {text} bytes; the most is {MAX_CHANGE_TEXT_BYTES}"),
            ));
        }
    }
    match (&change.unlinked, &change.merged) {
        (None, _) => Ok(()),
        (Some(_), Some(_)) => Err(SchemaError::new("unlinked", "a merged change stays linked")),
        (Some((by, at)), None) => {
            by.validate("unlinked/by")?;
            match change.created <= *at && *at <= change.updated {
                true => Ok(()),
                false => Err(SchemaError::new(
                    "unlinked/at",
                    "must be from when the change was linked to when it was updated",
                )),
            }
        }
    }
}

#[cfg(test)]
#[path = "change_tests.rs"]
mod tests;
