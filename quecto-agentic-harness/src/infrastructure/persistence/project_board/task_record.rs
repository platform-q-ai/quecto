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
use crate::domain::project_board::entities::task_parts::{
    Budget, Item, PlanStep, Review, Role, RoleLimits, Run, Team,
};
use crate::domain::project_board::value_objects::schema_error::SchemaError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{
    Effort, ItemState, RunOutcome, Severity, TaskKind, TaskStatus, Verdict,
};
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    plan: Vec<PlanStepRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    items: Vec<ItemRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    team: Option<TeamRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claim: Option<ClaimRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    runs: Vec<RunRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    reviews: Vec<ReviewRecord>,
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanStepRecord {
    title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    criteria: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemRecord {
    id: String,
    title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    criteria: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    depends_on: Vec<String>,
    state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    evidence: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamRecord {
    roles: Vec<RoleRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    budget: Option<BudgetRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deadline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    image: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RoleRecord {
    name: String,
    model: String,
    effort: String,
    limits: LimitsRecord,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LimitsRecord {
    members: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    turns: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetRecord {
    tokens: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RunRecord {
    id: String,
    started: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ended: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    outcome: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewRecord {
    id: String,
    round: u8,
    reviewer: IdentityRecord,
    at: String,
    severity: String,
    location: String,
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
    record.into_task().map_err(RecordError::Invalid)
}

/// The file bytes for `task`, at the current schema.
pub fn encode(task: &Task) -> Result<Vec<u8>, RecordError> {
    let mut bytes = serde_json::to_vec_pretty(&TaskRecord::from_task(task))
        .map_err(|error| RecordError::Malformed(error.to_string()))?;
    bytes.push(b'\n');
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
            plan: self
                .plan
                .into_iter()
                .map(PlanStepRecord::into_step)
                .collect(),
            items: indexed("items", self.items, ItemRecord::into_item)?,
            team: self.team.map(TeamRecord::into_team).transpose()?,
            claim: self.claim.map(ClaimRecord::into_claim).transpose()?,
            runs: indexed("runs", self.runs, RunRecord::into_run)?,
            reviews: indexed("reviews", self.reviews, ReviewRecord::into_review)?,
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
            plan: task.plan.iter().map(PlanStepRecord::from_step).collect(),
            items: task.items.iter().map(ItemRecord::from_item).collect(),
            team: task.team.as_ref().map(TeamRecord::from_team),
            claim: task.claim.as_ref().map(ClaimRecord::from_claim),
            runs: task.runs.iter().map(RunRecord::from_run).collect(),
            reviews: task.reviews.iter().map(ReviewRecord::from_review).collect(),
            prs: task.prs.clone(),
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

impl PlanStepRecord {
    fn into_step(self) -> PlanStep {
        PlanStep {
            title: self.title,
            criteria: self.criteria,
        }
    }

    fn from_step(step: &PlanStep) -> Self {
        Self {
            title: step.title.clone(),
            criteria: step.criteria.clone(),
        }
    }
}

impl ItemRecord {
    fn into_item(self, field: &str) -> Result<Item, SchemaError> {
        Ok(Item {
            id: slug(&format!("{field}/id"), &self.id)?,
            title: self.title,
            criteria: self.criteria,
            depends_on: slugs(&format!("{field}/depends_on"), &self.depends_on)?,
            state: word(&format!("{field}/state"), &self.state, ItemState::parse)?,
            owner: self.owner,
            evidence: self.evidence,
        })
    }

    fn from_item(item: &Item) -> Self {
        Self {
            id: item.id.as_str().into(),
            title: item.title.clone(),
            criteria: item.criteria.clone(),
            depends_on: item
                .depends_on
                .iter()
                .map(|slug| slug.as_str().into())
                .collect(),
            state: item.state.as_str().into(),
            owner: item.owner.clone(),
            evidence: item.evidence.clone(),
        }
    }
}

impl TeamRecord {
    fn into_team(self) -> Result<Team, SchemaError> {
        Ok(Team {
            roles: indexed("team/roles", self.roles, RoleRecord::into_role)?,
            budget: self.budget.map(|budget| Budget {
                tokens: budget.tokens,
            }),
            deadline: self
                .deadline
                .map(|deadline| time("team/deadline", &deadline))
                .transpose()?,
            image: self.image,
        })
    }

    fn from_team(team: &Team) -> Self {
        Self {
            roles: team.roles.iter().map(RoleRecord::from_role).collect(),
            budget: team.budget.as_ref().map(|budget| BudgetRecord {
                tokens: budget.tokens,
            }),
            deadline: team
                .deadline
                .as_ref()
                .map(|deadline| deadline.as_str().into()),
            image: team.image.clone(),
        }
    }
}

impl RoleRecord {
    fn into_role(self, field: &str) -> Result<Role, SchemaError> {
        Ok(Role {
            name: slug(&format!("{field}/name"), &self.name)?,
            model: self.model,
            effort: word(&format!("{field}/effort"), &self.effort, Effort::parse)?,
            limits: RoleLimits {
                members: self.limits.members,
                turns: self.limits.turns,
            },
        })
    }

    fn from_role(role: &Role) -> Self {
        Self {
            name: role.name.as_str().into(),
            model: role.model.clone(),
            effort: role.effort.as_str().into(),
            limits: LimitsRecord {
                members: role.limits.members,
                turns: role.limits.turns,
            },
        }
    }
}

impl RunRecord {
    fn into_run(self, field: &str) -> Result<Run, SchemaError> {
        let outcome =
            |outcome: String| word(&format!("{field}/outcome"), &outcome, RunOutcome::parse);
        Ok(Run {
            id: slug(&format!("{field}/id"), &self.id)?,
            started: time(&format!("{field}/started"), &self.started)?,
            ended: self
                .ended
                .map(|ended| time(&format!("{field}/ended"), &ended))
                .transpose()?,
            outcome: self.outcome.map(outcome).transpose()?,
        })
    }

    fn from_run(run: &Run) -> Self {
        Self {
            id: run.id.as_str().into(),
            started: run.started.as_str().into(),
            ended: run.ended.as_ref().map(|ended| ended.as_str().into()),
            outcome: run.outcome.map(|outcome| outcome.as_str().into()),
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
