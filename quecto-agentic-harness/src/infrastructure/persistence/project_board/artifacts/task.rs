//! A task's file (`quecto-task`, epic #2481 M2b-2). Its title is the
//! task's own; its fields and sections hold the rest, every list in the
//! task's own order:
//!
//! - Description, prose;
//! - Depends on `[Task]`, Superseded by `[Task]`;
//! - Pull requests `[Pull request]`, `#n`: the links made the old way,
//!   until S2c retires them;
//! - References `[Reference]`, code cells: `#n`, `owner/repo#n` or an
//!   https URL, which may hold a `_` or `%` an ASCII cell refuses.
//!
//! A missing field row or section is "not set" or "none". Claimed by,
//! Claimed since and Claim expires come together, or none of them. Review
//! findings are artifacts of their own (M1c): a task's file holds none.
use super::cells::{self, column, refused};
use super::{ArtifactCodec, CodecError};
use crate::domain::project_board::entities::claim::Claim;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::reference::{IssueNumber, Reference};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::{Resolution, TaskKind, TaskStatus};
use crate::infrastructure::persistence::project_board::markdown::{
    Body, BodyShape, CellKind, Column, Document, Limits, Section, SectionShape, Shape, blocks_of,
    check_title, text_of,
};

const TITLE: &str = "Title";
const ID: &str = "Id";
const KIND: &str = "Kind";
const STATUS: &str = "Status";
const PARENT: &str = "Parent";
const ACCEPTED_PLAN: &str = "Accepted plan";
const CLAIMED_BY: &str = "Claimed by";
const CLAIMED_SINCE: &str = "Claimed since";
const CLAIM_EXPIRES: &str = "Claim expires";
const GITHUB_ISSUE: &str = "GitHub issue";
const RESOLUTION: &str = "Resolution";
const CREATED: &str = "Created";
const UPDATED: &str = "Updated";
const DESCRIPTION: &str = "Description";
const DEPENDS_ON: &str = "Depends on";
const SUPERSEDED_BY: &str = "Superseded by";
const PULL_REQUESTS: &str = "Pull requests";
const REFERENCES: &str = "References";

const FIELDS: &[Column<'static>] = &[
    column(ID, CellKind::Ascii),
    column(KIND, CellKind::Ascii),
    column(STATUS, CellKind::Ascii),
    column(PARENT, CellKind::Ascii),
    column(ACCEPTED_PLAN, CellKind::Ascii),
    column(CLAIMED_BY, CellKind::Text),
    column(CLAIMED_SINCE, CellKind::Ascii),
    column(CLAIM_EXPIRES, CellKind::Ascii),
    column(GITHUB_ISSUE, CellKind::Ascii),
    column(RESOLUTION, CellKind::Ascii),
    column(CREATED, CellKind::Ascii),
    column(UPDATED, CellKind::Ascii),
];

const TASK_COLUMNS: &[Column<'static>] = &[column("Task", CellKind::Ascii)];
const PULL_REQUEST_COLUMNS: &[Column<'static>] = &[column("Pull request", CellKind::Ascii)];
const REFERENCE_COLUMNS: &[Column<'static>] = &[column("Reference", CellKind::Code)];

const SECTIONS: &[SectionShape<'static>] = &[
    SectionShape {
        heading: DESCRIPTION,
        body: BodyShape::Prose,
    },
    SectionShape {
        heading: DEPENDS_ON,
        body: BodyShape::Table(TASK_COLUMNS),
    },
    SectionShape {
        heading: SUPERSEDED_BY,
        body: BodyShape::Table(TASK_COLUMNS),
    },
    SectionShape {
        heading: PULL_REQUESTS,
        body: BodyShape::Table(PULL_REQUEST_COLUMNS),
    },
    SectionShape {
        heading: REFERENCES,
        body: BodyShape::Table(REFERENCE_COLUMNS),
    },
];

/// A task's file caps: the widest task the domain admits fits, measured
/// in `the_widest_task_fits_its_limits` beside the widest structure:
/// - the most lines, 65,725: a description of 65,536 characters as blank
///   lines inside a fence, and every table at its fullest;
/// - the longest line, 262,144 bytes: that description as one line of
///   four-byte letters;
/// - the most bytes, 346,706: the whole text budget with 11,093
///   description lines starting `#`, each written `\#` (one more line and
///   the references can no longer make the budget up).
pub const TASK_LIMITS: Limits = Limits {
    file_bytes: 512 * 1024,
    lines: 66_000,
    line_bytes: 264 * 1024,
};

pub struct TaskCodec;

impl ArtifactCodec for TaskCodec {
    type Value = Task;
    const SCHEMA: u32 = 1;
    const SHAPE: Shape<'static> = Shape {
        kind: "task",
        fields: FIELDS,
        sections: SECTIONS,
        limits: TASK_LIMITS,
    };

    fn to_document(task: &Task) -> Result<Document, CodecError> {
        let task = task.fields();
        // The heading's rules, here, so the error names the title.
        check_title(&task.title).map_err(|error| refused(TITLE, error.to_string()))?;
        let mut fields = vec![
            (ID, cells::slug(&task.id)),
            (KIND, cells::word(task.kind.as_str())),
            (STATUS, cells::word(task.status.as_str())),
        ];
        if let Some(parent) = &task.parent {
            fields.push((PARENT, cells::slug(parent)));
        }
        if let Some(version) = task.accepted_plan {
            fields.push((ACCEPTED_PLAN, cells::number(version.into())));
        }
        if let Some(claim) = &task.claim {
            fields.push((CLAIMED_BY, cells::identity(CLAIMED_BY, &claim.holder)?));
            fields.push((CLAIMED_SINCE, cells::time(&claim.since)));
            fields.push((CLAIM_EXPIRES, cells::time(&claim.expires)));
        }
        if let Some(issue) = task.github_issue {
            fields.push((GITHUB_ISSUE, cells::reference(GITHUB_ISSUE, issue.get())?));
        }
        if let Some(resolution) = task.resolution {
            fields.push((RESOLUTION, cells::word(resolution.as_str())));
        }
        fields.push((CREATED, cells::time(&task.created)));
        fields.push((UPDATED, cells::time(&task.updated)));
        let mut sections = vec![];
        // The description's own line, from 1, names where it is refused.
        let description = blocks_of(&task.description)
            .map_err(|error| refused(DESCRIPTION, error.to_string()))?;
        if let Some(description) = held(description) {
            sections.push(section(DESCRIPTION, Body::Prose(description)));
        }
        let slugs = |slugs: &[Slug]| slugs.iter().map(|slug| vec![cells::slug(slug)]).collect();
        let prs = task
            .prs
            .iter()
            .map(|pr| Ok(vec![cells::reference(PULL_REQUESTS, *pr)?]))
            .collect::<Result<Vec<_>, CodecError>>()?;
        let references = task
            .references
            .iter()
            .map(|reference| vec![reference.as_str().to_string()])
            .collect();
        for (heading, rows) in [
            (DEPENDS_ON, slugs(&task.depends_on)),
            (SUPERSEDED_BY, slugs(&task.superseded_by)),
            (PULL_REQUESTS, prs),
            (REFERENCES, references),
        ] {
            if let Some(rows) = held(rows) {
                sections.push(section(heading, Body::Table(rows)));
            }
        }
        debug_assert!(
            in_shape_order(fields.iter().map(|(name, _)| *name)),
            "the fields are written in the shape's order"
        );
        Ok(Document {
            schema: Self::SCHEMA,
            title: task.title.clone(),
            fields: fields
                .into_iter()
                .map(|(name, cell)| (name.into(), cell))
                .collect(),
            sections,
        })
    }

    fn from_document(document: &Document) -> Result<Task, CodecError> {
        let fields = cells::Fields::of(document);
        let field = |name: &str| fields.get(name);
        let required = |name: &'static str| fields.required(name);
        let claim = match (
            field(CLAIMED_BY),
            field(CLAIMED_SINCE),
            field(CLAIM_EXPIRES),
        ) {
            (Some(by), Some(since), Some(expires)) => Some(Claim {
                holder: cells::read_identity(CLAIMED_BY, by)?,
                since: cells::read_time(CLAIMED_SINCE, since)?,
                expires: cells::read_time(CLAIM_EXPIRES, expires)?,
            }),
            (None, None, None) => None,
            _ => {
                return Err(refused(
                    CLAIMED_BY,
                    "all or none of it, Claimed since and Claim expires",
                ));
            }
        };
        let accepted_plan = field(ACCEPTED_PLAN)
            .map(|cell| {
                let version = cells::read_number(ACCEPTED_PLAN, cell)?;
                u32::try_from(version).map_err(|_| refused(ACCEPTED_PLAN, "past u32"))
            })
            .transpose()?;
        let github_issue = field(GITHUB_ISSUE)
            .map(|cell| {
                let number = cells::read_reference(GITHUB_ISSUE, cell)?;
                IssueNumber::new(number).map_err(|error| refused(GITHUB_ISSUE, error.problem))
            })
            .transpose()?;
        let mut read = Read::default();
        for section in &document.sections {
            read.section(section)?;
        }
        let fields = TaskFields {
            id: cells::read_slug(ID, required(ID)?)?,
            title: document.title.clone(),
            kind: cells::read_word(KIND, required(KIND)?, TaskKind::parse)?,
            description: read.description,
            status: cells::read_word(STATUS, required(STATUS)?, TaskStatus::parse)?,
            parent: field(PARENT)
                .map(|cell| cells::read_slug(PARENT, cell))
                .transpose()?,
            depends_on: read.depends_on,
            accepted_plan,
            claim,
            prs: read.prs,
            github_issue,
            resolution: field(RESOLUTION)
                .map(|cell| cells::read_word(RESOLUTION, cell, Resolution::parse))
                .transpose()?,
            superseded_by: read.superseded_by,
            references: read.references,
            created: cells::read_time(CREATED, required(CREATED)?)?,
            updated: cells::read_time(UPDATED, required(UPDATED)?)?,
        };
        Task::new(fields)
            .map_err(|error| refused("Task", format!("{}: {}", error.field, error.problem)))
    }
}

/// Whether `names` come in [`FIELDS`]' order, each once.
fn in_shape_order<'a>(names: impl Iterator<Item = &'a str>) -> bool {
    let mut shape = FIELDS.iter();
    names
        .into_iter()
        .all(|name| shape.any(|field| field.name == name))
}

/// `entries`, when it holds any: a section only for what holds something
/// (a missing one is "none").
fn held<T>(entries: Vec<T>) -> Option<Vec<T>> {
    match entries.as_slice() {
        [_, ..] => Some(entries),
        [] => None,
    }
}

fn section(heading: &str, body: Body) -> Section {
    Section {
        heading: heading.into(),
        body,
    }
}

/// What a task's sections hold.
#[derive(Default)]
struct Read {
    description: String,
    depends_on: Vec<Slug>,
    superseded_by: Vec<Slug>,
    prs: Vec<u64>,
    references: Vec<Reference>,
}

impl Read {
    fn section(&mut self, section: &Section) -> Result<(), CodecError> {
        match (section.heading.as_str(), &section.body) {
            (DESCRIPTION, Body::Prose(blocks)) => self.description = text_of(blocks),
            (DEPENDS_ON, Body::Table(rows)) => self.depends_on = slugs(DEPENDS_ON, rows)?,
            (SUPERSEDED_BY, Body::Table(rows)) => {
                self.superseded_by = slugs(SUPERSEDED_BY, rows)?;
            }
            (PULL_REQUESTS, Body::Table(rows)) => {
                self.prs = rows
                    .iter()
                    .map(|row| cells::read_reference(PULL_REQUESTS, one(PULL_REQUESTS, row)?))
                    .collect::<Result<_, _>>()?;
            }
            (REFERENCES, Body::Table(rows)) => {
                self.references = rows
                    .iter()
                    .map(|row| {
                        Reference::parse(one(REFERENCES, row)?)
                            .map_err(|error| refused(REFERENCES, error.problem))
                    })
                    .collect::<Result<_, _>>()?;
            }
            _ => return Err(refused("Task", format!("a section {:?}", section.heading))),
        }
        Ok(())
    }
}

/// The one cell of a row of a one-column table.
fn one<'a>(table: &'static str, row: &'a [String]) -> Result<&'a str, CodecError> {
    match row {
        [cell] => Ok(cell),
        _ => Err(refused(table, "a row of one cell")),
    }
}

fn slugs(table: &'static str, rows: &[Vec<String>]) -> Result<Vec<Slug>, CodecError> {
    rows.iter()
        .map(|row| cells::read_slug(table, one(table, row)?))
        .collect()
}

#[cfg(test)]
#[path = "task_tests.rs"]
mod tests;
