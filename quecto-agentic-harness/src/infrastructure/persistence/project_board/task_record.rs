//! The file form of a task, `tasks/<id>.json` on a project's `quecto/board`
//! branch: pretty JSON with a final newline, keys in the order below.
//!
//! **Schema versions.** `schema` is the first key. A reader admits 1 to
//! [`SCHEMA_VERSION`] and refuses a newer file, asking for an upgrade; a
//! writer always writes [`SCHEMA_VERSION`], so it never lowers a file's.
//! Any new field or vocabulary word bumps [`SCHEMA_VERSION`].
//!
//! **Optional keys.** `schema`, `id`, `title`, `kind`, `status`, `created`
//! and `updated` are required. Every other key is written only when set
//! (non-empty), and a missing one means "not set".
//!
//! **Reading.** [`decode`] refuses a file over [`MAX_TASK_FILE_BYTES`]
//! before parsing it, then parses the bytes straight into the record, where
//! duplicate and unknown keys are refused. Never read a task file through
//! `serde_json::Value` or SQLite's json1, which keep the last of duplicate
//! keys. The store also checks that `id` matches the file name.
use crate::domain::project_board::entities::claim::{Claim, Identity};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::schema_error::SchemaError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_TASK_FILE_BYTES: usize = 256 * 1024;

/// Why a task file could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    TooLarge {
        bytes: usize,
    },
    /// Not JSON of the record's shape: a syntax error, a duplicate,
    /// unknown or missing key, or a value of the wrong JSON type.
    Malformed(String),
    NewerSchema {
        found: u32,
    },
    UnknownSchema {
        found: u32,
    },
    Invalid(SchemaError),
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { bytes } => write!(
                f,
                "task file is {bytes} bytes; the most is {MAX_TASK_FILE_BYTES}"
            ),
            Self::Malformed(reason) => write!(f, "task file is malformed: {reason}"),
            Self::NewerSchema { found } => write!(
                f,
                "this board's task file uses schema {found}, newer than quecto {} reads (schema {SCHEMA_VERSION}); upgrade quecto",
                env!("CARGO_PKG_VERSION")
            ),
            Self::UnknownSchema { found } => {
                write!(f, "task file schema {found} is not a known schema")
            }
            Self::Invalid(error) => write!(f, "task file breaks the schema: {error}"),
        }
    }
}

impl std::error::Error for RecordError {}

#[derive(Deserialize)]
struct SchemaProbe {
    schema: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskRecord {
    schema: u32,
    id: String,
    title: String,
    kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    description: String,
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claim: Option<ClaimRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    prs: Vec<u64>,
    created: String,
    updated: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimRecord {
    holder: IdentityRecord,
    since: String,
    expires: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityRecord {
    name: String,
    email: String,
}

/// The task a file holds, or why it holds none.
pub fn decode(bytes: &[u8]) -> Result<Task, RecordError> {
    let malformed = |error: serde_json::Error| RecordError::Malformed(error.to_string());
    let probe: SchemaProbe = serde_json::from_slice(bytes).map_err(malformed)?;
    match probe.schema {
        _ if probe.schema < u32::MAX => {}
        found if found > SCHEMA_VERSION => return Err(RecordError::NewerSchema { found }),
        found => return Err(RecordError::UnknownSchema { found }),
    }
    let record: TaskRecord = serde_json::from_slice(bytes).map_err(malformed)?;
    record.into_task().map_err(RecordError::Invalid)
}

/// The file bytes for `task`, at the current schema.
pub fn encode(task: &Task) -> Result<Vec<u8>, RecordError> {
    let mut bytes = serde_json::to_vec_pretty(&TaskRecord::from_task(task))
        .map_err(|error| RecordError::Malformed(error.to_string()))?;
    bytes.push(b'\n');
    if bytes.len() < usize::MAX {
        Ok(bytes)
    } else {
        Err(RecordError::TooLarge { bytes: bytes.len() })
    }
}

fn slug(field: &str, text: &str) -> Result<Slug, SchemaError> {
    Slug::parse(text).map_err(|error| error.at(field))
}

fn slugs(field: &str, texts: &[String]) -> Result<Vec<Slug>, SchemaError> {
    let indexed = texts.iter().enumerate();
    indexed
        .map(|(index, text)| slug(&format!("{field}/{index}"), text))
        .collect()
}

fn time(field: &str, text: &str) -> Result<Timestamp, SchemaError> {
    Timestamp::parse(text).map_err(|error| error.at(field))
}

fn word<T>(field: &str, text: &str, parse: fn(&str) -> Option<T>) -> Result<T, SchemaError> {
    parse(text).ok_or_else(|| SchemaError::new(field, format!("{text:?} is not a known value")))
}

impl TaskRecord {
    fn into_task(self) -> Result<Task, SchemaError> {
        Task::new(TaskFields {
            id: slug("id", &self.id)?,
            title: self.title,
            kind: word("kind", &self.kind, TaskKind::parse)?,
            description: self.description,
            status: word("status", &self.status, TaskStatus::parse)?,
            parent: self
                .parent
                .map(|parent| slug("parent", &parent))
                .transpose()?,
            depends_on: slugs("depends_on", &self.depends_on)?,
            claim: self.claim.map(ClaimRecord::into_claim).transpose()?,
            prs: self.prs,
            created: time("created", &self.created)?,
            updated: time("updated", &self.updated)?,
        })
    }

    fn from_task(task: &Task) -> Self {
        let task = task.fields();
        Self {
            schema: SCHEMA_VERSION,
            id: task.id.as_str().into(),
            title: task.title.clone(),
            kind: task.kind.as_str().into(),
            description: task.description.clone(),
            status: task.status.as_str().into(),
            parent: task.parent.as_ref().map(|parent| parent.as_str().into()),
            depends_on: task
                .depends_on
                .iter()
                .map(|slug| slug.as_str().into())
                .collect(),
            claim: task.claim.as_ref().map(ClaimRecord::from_claim),
            prs: task.prs.clone(),
            created: task.created.as_str().into(),
            updated: task.updated.as_str().into(),
        }
    }
}

impl ClaimRecord {
    fn into_claim(self) -> Result<Claim, SchemaError> {
        Ok(Claim {
            holder: Identity {
                name: self.holder.name,
                email: self.holder.email,
            },
            since: time("claim/since", &self.since)?,
            expires: time("claim/expires", &self.expires)?,
        })
    }

    fn from_claim(claim: &Claim) -> Self {
        Self {
            holder: IdentityRecord {
                name: claim.holder.name.clone(),
                email: claim.holder.email.clone(),
            },
            since: claim.since.as_str().into(),
            expires: claim.expires.as_str().into(),
        }
    }
}

#[cfg(test)]
#[path = "task_record_tests.rs"]
mod tests;
