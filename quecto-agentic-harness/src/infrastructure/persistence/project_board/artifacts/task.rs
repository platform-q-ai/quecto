//! A task's file (`quecto-task`, epic #2481 M2b-2). Its title is the
//! task's own; its fields and sections hold the rest, every list in the
//! task's own order:
//!
//! - at schema 1, Description, prose: legacy text, as written; or, at
//!   schema 2 (a description kept in its sections, #2705 T2), a section
//!   each in its order: Goal and Why, prose; Done when `[Done | Outcome]`,
//!   `yes` or `no`; Links, prose (`- [text](https://…)` lines). The task's
//!   description form picks the schema, never its text; a file holding a
//!   Reason (a status move's, #2705 T4) is at schema 3, either layout;
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
use crate::domain::project_board::entities::task_parts::{
    Description, DescriptionForm, EXCEPTIONS, Outcome, PARTS, Shape as PartShape,
};
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
const REASON: &str = "Reason";
const CREATED: &str = "Created";
const UPDATED: &str = "Updated";
const DESCRIPTION: &str = "Description";
const DEPENDS_ON: &str = "Depends on";
const SUPERSEDED_BY: &str = "Superseded by";
const PULL_REQUESTS: &str = "Pull requests";
const REFERENCES: &str = "References";
/// Why a file's description sections are refused: they make no
/// description.
const NOT_A_DESCRIPTION: &str = "its sections must be a Goal paragraph, an optional Why \
     paragraph, a Done when table of outcomes, and optional Links written as \"- \" lines";
/// Why a schema-2 file holding a Description beside the sections is
/// refused.
const BESIDE_SECTIONS: &str = "beside its own sections";
/// Why a schema-1 file holding the description's sections is refused: no
/// build wrote them at schema 1.
const SECTIONS_AT_SCHEMA_1: &str = "its sections are written only at schema 2";

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
    column(REASON, CellKind::Text),
    column(CREATED, CellKind::Ascii),
    column(UPDATED, CellKind::Ascii),
];

const TASK_COLUMNS: &[Column<'static>] = &[column("Task", CellKind::Ascii)];
const PULL_REQUEST_COLUMNS: &[Column<'static>] = &[column("Pull request", CellKind::Ascii)];
const REFERENCE_COLUMNS: &[Column<'static>] = &[column("Reference", CellKind::Code)];
/// The done-when outcomes: a table, as a reader would take `- [ ]` in
/// prose for a checkbox the prose codec does not hold.
const DONE_WHEN: &str = PARTS[2].heading;
const OUTCOME_COLUMNS: &[Column<'static>] = &[
    column("Done", CellKind::Ascii),
    column("Outcome", CellKind::Text),
];
const DONE: &str = "yes";
const OPEN: &str = "no";
/// The schema of a file whose description is kept in its sections (#2705
/// T2). A legacy description stays at schema 1, the bytes earlier builds
/// wrote; an earlier build reads schema 2 as a newer quecto's ("upgrade
/// quecto to read it"), never as a file it misreads.
const SECTIONED_SCHEMA: u32 = 2;
/// The schema of a file holding a status move's reason (#2705 T4), its
/// description kept either way (its sections, or legacy text as its
/// Description): an earlier build reads it as a newer quecto's.
const REASONED_SCHEMA: u32 = 3;
const EXCEPTION_SCHEMA: u32 = 4;
const EXCEPTION_COLUMNS: &[Column<'static>] = &[
    column("Outcome index", CellKind::Ascii),
    column("Reason", CellKind::Text),
    column("By", CellKind::Text),
    column("At", CellKind::Ascii),
];

const SECTIONS: &[SectionShape<'static>] = &[
    SectionShape {
        heading: DESCRIPTION,
        body: BodyShape::Prose,
    },
    SectionShape {
        heading: PARTS[0].heading,
        body: BodyShape::Prose,
    },
    SectionShape {
        heading: PARTS[1].heading,
        body: BodyShape::Prose,
    },
    SectionShape {
        heading: DONE_WHEN,
        body: BodyShape::Table(OUTCOME_COLUMNS),
    },
    SectionShape {
        heading: EXCEPTIONS,
        body: BodyShape::Table(EXCEPTION_COLUMNS),
    },
    SectionShape {
        heading: PARTS[3].heading,
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
    const SCHEMA: u32 = EXCEPTION_SCHEMA;
    const OLDEST_SCHEMA: u32 = 1;
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
        if let Some(reason) = &task.reason {
            fields.push((REASON, reason.clone()));
        }
        fields.push((CREATED, cells::time(&task.created)));
        fields.push((UPDATED, cells::time(&task.updated)));
        let mut sections = vec![];
        // The description's form, never its text's shape, picks the
        // layout (#2705 T2): one kept in its sections is a file section
        // each, at schema 2; any other stays the Description, as written,
        // at schema 1, the bytes earlier builds wrote. Each one's own
        // line, from 1, names where it is refused.
        let schema = match task.description_form {
            DescriptionForm::Sectioned => {
                let description = Description::read(&task.description)
                    .ok_or_else(|| refused(DESCRIPTION, NOT_A_DESCRIPTION))?;
                for (heading, text) in description.sections() {
                    match heading {
                        DONE_WHEN => sections.push(section(heading, outcome_rows(&description))),
                        EXCEPTIONS => sections
                            .push(section(heading, Body::Table(description.exception_rows()))),
                        _ => sections.extend(prose(heading, &text)?),
                    }
                }
                SECTIONED_SCHEMA
            }
            DescriptionForm::Legacy => {
                sections.extend(prose(DESCRIPTION, &task.description)?);
                Self::OLDEST_SCHEMA
            }
        };
        // A reason (#2705 T4) is written at schema 3, whatever the form, so
        // an earlier build reads the file as a newer quecto's.
        let schema = match task.reason {
            Some(_) => REASONED_SCHEMA,
            None => schema,
        };
        let schema = match Description::read(&task.description) {
            Some(description)
                if task.description_form == DescriptionForm::Sectioned
                    && matches!(description.exception_rows().as_slice(), [_, ..]) =>
            {
                EXCEPTION_SCHEMA
            }
            _ => schema,
        };
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
            schema,
            title: task.title.clone(),
            fields: fields
                .into_iter()
                .map(|(name, cell)| (name.into(), cell))
                .collect(),
            sections,
        })
    }

    fn compatible_read(mut document: Document) -> Document {
        // Only the implicit checkboxes of sectioned Done tasks have new read
        // semantics. The original document has already passed parsing and domain
        // validation. Never reinterpret legacy prose or unrelated archives.
        let done = document
            .fields
            .iter()
            .any(|(key, value)| key == STATUS && value == "done");
        let sectioned = document
            .sections
            .iter()
            .any(|section| section.heading == DONE_WHEN);
        if done && sectioned {
            let explicit: Vec<String> = document
                .sections
                .iter()
                .filter(|section| section.heading == EXCEPTIONS)
                .flat_map(|section| match &section.body {
                    Body::Table(rows) => rows.iter().map(|row| row[0].clone()).collect(),
                    Body::Prose(_) => vec![],
                })
                .collect();
            for section in &mut document.sections {
                if let (DONE_WHEN, Body::Table(rows)) =
                    (section.heading.as_str(), &mut section.body)
                {
                    for (index, row) in rows.iter_mut().enumerate() {
                        if explicit.contains(&index.to_string()) {
                        } else {
                            row[0] = DONE.into();
                        }
                    }
                }
            }
        }
        document
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
        let (description, description_form) = read.description(document.schema)?;
        let fields = TaskFields {
            id: cells::read_slug(ID, required(ID)?)?,
            title: document.title.clone(),
            kind: cells::read_word(KIND, required(KIND)?, TaskKind::parse)?,
            description,
            description_form,
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
            reason: field(REASON).map(str::to_string),
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

/// `text` as a prose section headed `heading`, when it holds any; its
/// own line, from 1, names where it is refused.
fn prose(heading: &'static str, text: &str) -> Result<Option<Section>, CodecError> {
    let blocks = blocks_of(text).map_err(|error| refused(heading, error.to_string()))?;
    Ok(held(blocks).map(|blocks| section(heading, Body::Prose(blocks))))
}

/// The description's outcomes as rows: done or not, and the outcome.
fn outcome_rows(description: &Description) -> Body {
    let rows = description.done_when().iter().map(|outcome| {
        let done = match outcome.done {
            true => DONE,
            false => OPEN,
        };
        vec![cells::word(done), outcome.text.clone()]
    });
    Body::Table(rows.collect())
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
    /// The description's own sections, each a heading and its text.
    parts: Vec<(&'static str, String)>,
    depends_on: Vec<Slug>,
    superseded_by: Vec<Slug>,
    prs: Vec<u64>,
    references: Vec<Reference>,
    exceptions: Vec<Vec<String>>,
}

impl Read {
    /// The schema a legacy description is read at.
    const LEGACY_SCHEMA: u32 = <TaskCodec as ArtifactCodec>::OLDEST_SCHEMA;

    /// The task's description and its form, by the file's `schema`: at
    /// schema 1 its Description as written (legacy); at schema 2 the text
    /// its own sections make, which must be a description in its sections;
    /// at schema 3 (a reason, #2705 T4) either, by the layout it holds.
    /// Never both, and never the other schema's layout.
    fn description(&mut self, schema: u32) -> Result<(String, DescriptionForm), CodecError> {
        let parts = std::mem::take(&mut self.parts);
        let text = std::mem::take(&mut self.description);
        if self.exceptions.is_empty() {
        } else {
            match schema {
                EXCEPTION_SCHEMA => {}
                _ => return Err(refused(EXCEPTIONS, "exceptions require schema 4")),
            }
        }
        // A schema-3 file is read as the layout it holds: sections, or a
        // Description (or none, legacy text left empty).
        let schema = match (schema, parts.as_slice()) {
            (REASONED_SCHEMA | EXCEPTION_SCHEMA, [_, ..]) => SECTIONED_SCHEMA,
            (REASONED_SCHEMA | EXCEPTION_SCHEMA, []) => Self::LEGACY_SCHEMA,
            (schema, _) => schema,
        };
        match (schema, text.is_empty(), parts.as_slice()) {
            (SECTIONED_SCHEMA, true, [_, ..]) => {
                let pairs = parts
                    .iter()
                    .map(|(heading, text)| (*heading, text.as_str()));
                let text = Description::assemble(pairs);
                let text = match Description::read(&text) {
                    Some(mut description) => {
                        description
                            .apply_exception_rows(&self.exceptions)
                            .map_err(|error| refused(EXCEPTIONS, error.to_string()))?;
                        description.markdown()
                    }
                    None => return Err(refused(DESCRIPTION, NOT_A_DESCRIPTION)),
                };
                match Description::read(&text) {
                    Some(_) => Ok((text, DescriptionForm::Sectioned)),
                    None => Err(refused(DESCRIPTION, NOT_A_DESCRIPTION)),
                }
            }
            (SECTIONED_SCHEMA, false, _) => Err(refused(DESCRIPTION, BESIDE_SECTIONS)),
            (SECTIONED_SCHEMA, true, []) => Err(refused(DESCRIPTION, NOT_A_DESCRIPTION)),
            (_, _, []) => Ok((text, DescriptionForm::Legacy)),
            (_, _, [_, ..]) => Err(refused(DESCRIPTION, SECTIONS_AT_SCHEMA_1)),
        }
    }

    fn section(&mut self, section: &Section) -> Result<(), CodecError> {
        let part = PARTS.iter().find(|rule| rule.heading == section.heading);
        let prose_part =
            part.filter(|rule| matches!(rule.shape, PartShape::Paragraph | PartShape::List));
        if let (Some(rule), Body::Prose(blocks)) = (prose_part, &section.body) {
            self.parts.push((rule.heading, text_of(blocks)));
            return Ok(());
        }
        match (section.heading.as_str(), &section.body) {
            (DESCRIPTION, Body::Prose(blocks)) => self.description = text_of(blocks),
            (DONE_WHEN, Body::Table(rows)) => {
                let outcomes = rows
                    .iter()
                    .map(|row| match row.as_slice() {
                        [done, text] => Ok(Outcome {
                            done: cells::read_word(DONE_WHEN, done, |word| match word {
                                DONE => Some(true),
                                OPEN => Some(false),
                                _ => None,
                            })?,
                            text: text.clone(),
                            exception: None,
                        }),
                        _ => Err(refused(DONE_WHEN, "a row of two cells")),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                self.parts
                    .push((DONE_WHEN, Description::checklist(&outcomes)));
            }
            (EXCEPTIONS, Body::Table(rows)) => self.exceptions = rows.clone(),
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

#[cfg(test)]
#[path = "task_sections_tests.rs"]
mod sections_tests;

#[cfg(test)]
#[path = "task_reason_tests.rs"]
mod reason_tests;
