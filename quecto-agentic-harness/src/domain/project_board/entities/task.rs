//! A task: the overall job a project plans, readies and claims. A [`Task`]
//! is only ever built from fields that pass [`Task::new`], so an invalid
//! task cannot exist; change one by taking its fields and building anew.
use super::super::value_objects::board_text;
use super::super::value_objects::reference::{IssueNumber, Points, Reference};
use super::super::value_objects::schema_error::{SchemaError, distinct, line, markdown};
use super::super::value_objects::slug::Slug;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::{Resolution, TaskKind, TaskStatus};
use super::claim::Claim;
use super::plan::MAX_PLAN_VERSION;

/// GitHub's issue title limit.
pub const MAX_TITLE_CHARS: usize = 256;
/// GitHub's issue body limit.
pub const MAX_DESCRIPTION_CHARS: usize = 65_536;
pub const MAX_DEPENDENCIES: usize = 64;
pub const MAX_PRS: usize = 16;
/// The most tasks one task is superseded by.
pub const MAX_SUPERSEDED_BY: usize = 16;
/// The most references a task keeps.
pub const MAX_REFERENCES: usize = 64;
/// All of a task's text together, in UTF-8 bytes: this budget keeps every
/// task the domain admits inside the file it is stored in (the record
/// layer proves it). Its title and description alone always fit (a
/// character is at most four bytes); with its references they can add up
/// past it, so the budget is checked.
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
    /// The host issue the task came from or is mirrored by.
    pub github_issue: Option<IssueNumber>,
    /// How the task was resolved: only a done or archived task is.
    pub resolution: Option<Resolution>,
    /// The tasks that took over its work.
    pub superseded_by: Vec<Slug>,
    /// Plain pointers to any issue, pull request or page, on the board or
    /// not (`#n`, `owner/repo#n`, an https URL): closed work, work outside
    /// the board, or what delivered the task (as #2265 was delivered by an
    /// on-board task). They are not dependencies.
    pub references: Vec<Reference>,
    pub created: Timestamp,
    pub updated: Timestamp,
}

impl TaskFields {
    /// The UTF-8 bytes of every free text in the task: its title and
    /// description, and its references. Ids, words, numbers, times and the
    /// claim's holder are not text: their size is bounded by the field
    /// limits, so taking a claim never runs a task over its budget. Review
    /// findings are review artifacts of their own (M1c), with their own
    /// budget.
    pub fn text_bytes(&self) -> usize {
        let references: usize = self
            .references
            .iter()
            .map(|reference| reference.as_str().len())
            .sum();
        self.title.len() + self.description.len() + references
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

    /// A task built from text as it enters the board (the write path):
    /// its title and description must be normalised board text, so no
    /// entry point skips normalising. Reading a task back (`new`) takes
    /// what earlier builds stored.
    ///
    /// # Errors
    /// Text that was never normalised, or what `new` refuses.
    pub fn entered(fields: TaskFields) -> Result<Self, SchemaError> {
        board_text::entered_line("title", &fields.title)?;
        board_text::entered_text("description", &fields.description)?;
        Self::new(fields)
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
    validate_closure(task)?;
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

/// Where the task came from and how it ended:
/// - its issue ([`IssueNumber`]), never one of its own pull requests (the
///   host numbers both from one sequence; a pull request that clashes is
///   reported where it is listed, `prs`);
/// - its resolution: none until it is done or archived; a done task's is
///   `completed` (an older file's done task is read as completed), and it
///   is superseded by none; an archived task's any;
/// - `superseded` and `duplicate` name what took it over, a task in
///   `superseded_by` or a reference (a closed issue is only a reference),
///   and a task superseded by others is unresolved, `superseded` or
///   `duplicate`;
/// - the tasks that superseded it, never itself;
/// - its references, each named once (by its key), any issue, pull request
///   or page, on the board or not, but never its own issue or one of its
///   own pull requests by `#n`.
///
/// The board's own checks are the importer's and M4's, through
/// [`issue_numbers_hold`](super::super::services::issue_numbers::issue_numbers_hold):
/// no two tasks share an issue (`issue_shared`), no issue is a pull request
/// on the board (`issue_is_pr`), and no task names by `#n` the pull request
/// of its own linked change (`own_pr_referenced`); and `superseded_by`
/// names tasks the board holds, without cycles.
fn validate_closure(task: &TaskFields) -> Result<(), SchemaError> {
    let issue = task.github_issue.map(IssueNumber::get);
    match task.prs.iter().position(|pr| Some(*pr) == issue) {
        None => {}
        Some(index) => {
            return Err(SchemaError::new(
                format!("prs/{index}"),
                "is the task's own issue: the host numbers issues and pull requests from one \
                 sequence",
            ));
        }
    }
    resolution_holds(task)?;
    distinct("superseded_by", &task.superseded_by, MAX_SUPERSEDED_BY)?;
    match task
        .superseded_by
        .iter()
        .position(|other| *other == task.id)
    {
        None => {}
        Some(index) => {
            return Err(SchemaError::new(
                format!("superseded_by/{index}"),
                "a task is not superseded by itself",
            ));
        }
    }
    let keys: Vec<&str> = task.references.iter().map(Reference::key).collect();
    distinct("references", &keys, MAX_REFERENCES)?;
    let own = |reference: &Reference| match reference.points() {
        Points::Own(number) => Some(number.get()) == issue || task.prs.contains(&number.get()),
        Points::Elsewhere(_) | Points::Page => false,
    };
    match task.references.iter().position(own) {
        None => Ok(()),
        Some(index) => Err(SchemaError::new(
            format!("references/{index}"),
            "is the task's own issue or pull request",
        )),
    }
}

/// The task's resolution fits its status and what superseded it.
fn resolution_holds(task: &TaskFields) -> Result<(), SchemaError> {
    use Resolution::{Completed, Duplicate, NotPlanned, Superseded};
    use TaskStatus::{Archived, Blocked, Claimed, Done, Draft, InProgress, Ready, Review};
    let status = task.status;
    let by_status = match (task.resolution, status) {
        (Some(Completed), Done)
        | (Some(_), Archived)
        | (None, Draft | Ready | Claimed | InProgress | Review | Blocked | Archived) => Ok(()),
        (_, Done) => Err("a done task is completed"),
        (Some(_), _) => Err("only a done or archived task is resolved"),
    };
    let named = !task.superseded_by.is_empty() || !task.references.is_empty();
    let by_successor = match (task.resolution, named, task.superseded_by.is_empty()) {
        (Some(Superseded | Duplicate), false, _) => {
            Err("a superseded or duplicate task names what took it over: a task or a reference")
        }
        (Some(Completed | NotPlanned), _, false) => {
            Err("a task superseded by others is unresolved, superseded or duplicate")
        }
        (None | Some(Completed | NotPlanned), _, true) | (None, _, false) => Ok(()),
        (Some(Superseded | Duplicate), true, _) => Ok(()),
    };
    match by_status.and(by_successor) {
        Ok(()) => Ok(()),
        Err(why) => Err(SchemaError::new(
            "resolution",
            format!("{why} (status {})", status.as_str()),
        )),
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

#[cfg(test)]
#[path = "task_closure_tests.rs"]
mod closure_tests;
