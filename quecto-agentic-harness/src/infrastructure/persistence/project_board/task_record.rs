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
use crate::domain::project_board::entities::task_parts::Review;
use crate::domain::project_board::value_objects::reference::{IssueNumber, Reference};
use crate::domain::project_board::value_objects::schema_error::SchemaError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{
    Resolution, Severity, TaskKind, TaskStatus, Verdict,
};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Each schema this quecto reads, oldest first, with the quecto release
/// that introduced it. A new schema adds a row here. Schema 1 is the
/// format as the task parts complete it (#2483, 0.107.251). Schema 2 is
/// the artifact model's slimmer task (#2481 M1a): its plan, items, team and
/// runs are artifacts of their own, and it names its `accepted_plan`; a
/// schema-1 file holding any of those is malformed to this reader. Schema
/// 3 adds where a task came from and how it ended (#2481 S8):
/// `github_issue`, `resolution`, `superseded_by` and `references`; an
/// older file holding a value in any of them is malformed. As for every
/// optional key, `null` in a single value and an empty list hold no
/// value: they read as not set, in any schema (a list set to `null` is
/// malformed, as every list is).
pub const SCHEMAS: &[(u32, &str)] = &[(1, "0.107.251"), (2, "0.107.274"), (3, "0.107.292")];
/// The schema every task file is written at: the last row of [`SCHEMAS`].
pub const SCHEMA_VERSION: u32 = 3;
const _: () = assert!(SCHEMAS[SCHEMAS.len() - 1].0 == SCHEMA_VERSION);
/// The largest task file read or written. Every task the domain admits is
/// written under it: the widest structure (about 300 KB) plus the whole
/// text budget at its widest escape (2 × `MAX_TEXT_BYTES`) stays below
/// 1 MiB, so a write is never refused only because the field limits add
/// up past the cap (`size_tests`).
pub const MAX_TASK_FILE_BYTES: usize = 1024 * 1024;

/// The quecto release that introduced `schema`, when this quecto knows it.
pub fn introduced_in(schema: u32) -> Option<&'static str> {
    let row = SCHEMAS.iter().find(|(number, _)| *number == schema);
    row.map(|(_, release)| *release)
}

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
            // A newer schema came with a release after this one; that
            // release is the least that reads it, and this quecto cannot
            // know its number.
            Self::NewerSchema { found } => write!(
                f,
                "this board's task file uses schema {found}, which needs a quecto newer than {} \
                 (it reads schemas 1 to {SCHEMA_VERSION}; schema {SCHEMA_VERSION} arrived in quecto {}); \
                 upgrade quecto",
                env!("CARGO_PKG_VERSION"),
                introduced_in(SCHEMA_VERSION).unwrap_or("?"),
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
    accepted_plan: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claim: Option<ClaimRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    reviews: Vec<ReviewRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    prs: Vec<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    github_issue: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    superseded_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    references: Vec<String>,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewRecord {
    id: String,
    round: u8,
    reviewer: IdentityRecord,
    at: String,
    severity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    location: Option<String>,
    finding: String,
    verdict: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    proving_test: Option<String>,
}

/// The task a file holds, or why it holds none.
pub fn decode(bytes: &[u8]) -> Result<Task, RecordError> {
    if bytes.len() > MAX_TASK_FILE_BYTES {
        return Err(RecordError::TooLarge { bytes: bytes.len() });
    }
    let malformed = |error: serde_json::Error| RecordError::Malformed(error.to_string());
    let probe: SchemaProbe = serde_json::from_slice(bytes).map_err(malformed)?;
    match probe.schema {
        1..=SCHEMA_VERSION => {}
        found if found > SCHEMA_VERSION => return Err(RecordError::NewerSchema { found }),
        found => return Err(RecordError::UnknownSchema { found }),
    }
    let record: TaskRecord = serde_json::from_slice(bytes).map_err(malformed)?;
    // Each key's values belong to the schemas that have it: `accepted_plan`
    // arrived in schema 2, and where a task came from and how it ended in
    // schema 3. A key holding no value (`null` in a single value, an empty
    // list) reads as not set, in any schema, as every optional key does.
    match (record.schema, &record.accepted_plan) {
        (2.., _) | (_, None) => {}
        (_, Some(_)) => {
            return Err(RecordError::Malformed(
                "accepted_plan arrived in schema 2".into(),
            ));
        }
    }
    let closure = record.github_issue.is_some()
        || record.resolution.is_some()
        || !record.superseded_by.is_empty()
        || !record.references.is_empty();
    match (record.schema, closure) {
        (3.., _) | (_, false) => {}
        (_, true) => {
            return Err(RecordError::Malformed(
                "github_issue, resolution, superseded_by and references arrived in schema 3".into(),
            ));
        }
    }
    // A done task is completed. One written before schema 3 has no
    // resolution: it is read as completed, as it ended. An archived one
    // keeps none: whether it was done or dropped first is not known.
    let mut record = record;
    let upgraded = matches!(
        (record.schema, record.status.as_str(), &record.resolution),
        (1..=2, "done", None)
    );
    record.resolution = match upgraded {
        true => Some(Resolution::Completed.as_str().into()),
        false => record.resolution,
    };
    record.into_task().map_err(RecordError::Invalid)
}

/// The file bytes for `task`, at the current schema.
pub fn encode(task: &Task) -> Result<Vec<u8>, RecordError> {
    let mut bytes = serde_json::to_vec_pretty(&TaskRecord::from_task(task))
        .map_err(|error| RecordError::Malformed(error.to_string()))?;
    bytes.push(b'\n');
    debug_assert!(
        bytes.len() <= MAX_TASK_FILE_BYTES,
        "every valid task fits its file"
    );
    if bytes.len() <= MAX_TASK_FILE_BYTES {
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

/// Each record mapped with its index in the field's path.
fn indexed<R, T>(
    field: &str,
    records: Vec<R>,
    map: fn(R, &str) -> Result<T, SchemaError>,
) -> Result<Vec<T>, SchemaError> {
    let indexed = records.into_iter().enumerate();
    indexed
        .map(|(index, record)| map(record, &format!("{field}/{index}")))
        .collect()
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
            accepted_plan: self.accepted_plan,
            claim: self.claim.map(ClaimRecord::into_claim).transpose()?,
            reviews: indexed("reviews", self.reviews, ReviewRecord::into_review)?,
            prs: self.prs,
            github_issue: self
                .github_issue
                .map(|number| IssueNumber::new(number).map_err(|error| error.at("github_issue")))
                .transpose()?,
            resolution: self
                .resolution
                .map(|word_| word("resolution", &word_, Resolution::parse))
                .transpose()?,
            superseded_by: slugs("superseded_by", &self.superseded_by)?,
            references: indexed("references", self.references, |text, field| {
                Reference::parse(&text).map_err(|error| error.at(field))
            })?,
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
            accepted_plan: task.accepted_plan,
            claim: task.claim.as_ref().map(ClaimRecord::from_claim),
            reviews: task.reviews.iter().map(ReviewRecord::from_review).collect(),
            prs: task.prs.clone(),
            github_issue: task.github_issue.map(IssueNumber::get),
            resolution: task.resolution.map(|resolution| resolution.as_str().into()),
            superseded_by: task
                .superseded_by
                .iter()
                .map(|slug| slug.as_str().into())
                .collect(),
            references: task
                .references
                .iter()
                .map(|reference| reference.as_str().into())
                .collect(),
            created: task.created.as_str().into(),
            updated: task.updated.as_str().into(),
        }
    }
}

impl ClaimRecord {
    fn into_claim(self) -> Result<Claim, SchemaError> {
        Ok(Claim {
            holder: self.holder.into_identity(),
            since: time("claim/since", &self.since)?,
            expires: time("claim/expires", &self.expires)?,
        })
    }

    fn from_claim(claim: &Claim) -> Self {
        Self {
            holder: IdentityRecord::from_identity(&claim.holder),
            since: claim.since.as_str().into(),
            expires: claim.expires.as_str().into(),
        }
    }
}

impl IdentityRecord {
    fn into_identity(self) -> Identity {
        Identity {
            name: self.name,
            email: self.email,
        }
    }

    fn from_identity(identity: &Identity) -> Self {
        Self {
            name: identity.name.clone(),
            email: identity.email.clone(),
        }
    }
}

impl ReviewRecord {
    fn into_review(self, field: &str) -> Result<Review, SchemaError> {
        Ok(Review {
            id: slug(&format!("{field}/id"), &self.id)?,
            round: self.round,
            reviewer: self.reviewer.into_identity(),
            at: time(&format!("{field}/at"), &self.at)?,
            severity: word(
                &format!("{field}/severity"),
                &self.severity,
                Severity::parse,
            )?,
            location: self.location,
            finding: self.finding,
            verdict: word(&format!("{field}/verdict"), &self.verdict, Verdict::parse)?,
            proving_test: self.proving_test,
        })
    }

    fn from_review(review: &Review) -> Self {
        Self {
            id: review.id.as_str().into(),
            round: review.round,
            reviewer: IdentityRecord::from_identity(&review.reviewer),
            at: review.at.as_str().into(),
            severity: review.severity.as_str().into(),
            location: review.location.clone(),
            finding: review.finding.clone(),
            verdict: review.verdict.as_str().into(),
            proving_test: review.proving_test.clone(),
        }
    }
}

#[cfg(test)]
#[path = "task_record_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "task_record_size_tests.rs"]
mod size_tests;
