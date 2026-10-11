//! A change: a pointer to the code that delivers a task (#2481's artifact
//! model). It names a branch, the pull request while GitHub hosts pull
//! requests, and the merge commit once merged; commits, CI and merge state
//! are read live from git and GitHub, never copied. Its parent is the
//! task. An unlinked change (a wrong or abandoned one) is kept, marked, so
//! nothing is deleted.
use super::super::value_objects::schema_error::SchemaError;
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::claim::Identity;
use regex::Regex;
use std::sync::LazyLock;

/// The most changes a task links.
pub const MAX_CHANGE_NUMBER: u32 = 999;
/// The longest branch name.
pub const MAX_BRANCH_BYTES: usize = 255;
/// A change's text (its branch name): bounded by the field, and budgeted
/// like every artifact's.
pub const MAX_CHANGE_TEXT_BYTES: usize = MAX_BRANCH_BYTES;

/// A branch name: `/`-separated components of letters, digits, `.`, `_`
/// and `-`, each starting with a letter or digit; no `..` and no trailing
/// `.lock` (git's own refusals among those characters).
static BRANCH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*(?:/[A-Za-z0-9][A-Za-z0-9._-]*)*$").expect("branch")
});

/// Whether `text` is a full commit id: 40 (SHA-1) or 64 (SHA-256)
/// lowercase hex digits.
pub fn commit_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A change's content, unchecked: what [`Change::new`] validates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeFields {
    /// The task it delivers.
    pub task: Slug,
    /// From 1, per task.
    pub number: u32,
    pub branch: String,
    /// The pull request, from 1, while GitHub hosts pull requests.
    pub pr: Option<u64>,
    /// The merge commit, once merged: a branch may be deleted or
    /// rewritten, a commit id stays.
    pub merged: Option<String>,
    pub linked_by: Identity,
    /// Who unlinked it and when: a wrong or abandoned change, kept.
    pub unlinked: Option<(Identity, Timestamp)>,
    pub created: Timestamp,
    pub updated: Timestamp,
}

impl ChangeFields {
    /// The UTF-8 bytes of the change's free text: its branch name.
    pub fn text_bytes(&self) -> usize {
        self.branch.len()
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
    if change.created > change.updated {
        return Err(SchemaError::new("updated", "must not be before created"));
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
    let branch = &change.branch;
    let named = branch.len() <= MAX_BRANCH_BYTES
        && BRANCH.is_match(branch)
        && !branch.contains("..")
        && !branch.ends_with(".lock");
    match named {
        true => {}
        false => {
            return Err(SchemaError::new(
                "branch",
                format!("{branch:?} is not a branch name of at most {MAX_BRANCH_BYTES} bytes"),
            ));
        }
    }
    match change.pr.is_none_or(|pr| pr >= 1) {
        true => {}
        false => return Err(SchemaError::new("pr", "a pull request number starts at 1")),
    }
    match change.merged.as_deref().is_none_or(commit_id) {
        true => {}
        false => return Err(SchemaError::new("merged", "must be a full commit id")),
    }
    change.linked_by.validate("linked_by")?;
    match &change.unlinked {
        None => Ok(()),
        Some(_) if change.merged.is_some() => {
            Err(SchemaError::new("unlinked", "a merged change stays linked"))
        }
        Some((by, at)) => {
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
