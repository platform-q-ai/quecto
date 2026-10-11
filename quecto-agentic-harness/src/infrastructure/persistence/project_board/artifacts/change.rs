//! A change's file (`quecto-change`, epic #2481 M2c-1): fields only, its
//! title derived, `Change {number} of {task}`. Open, merged or closed is
//! read live from the host, never stored. A missing optional row is "not
//! set"; Unlinked by and Unlinked at come together.
//!
//! The Repository and Branch cells are code: a repository may be named
//! `org/www.site` or `_x/y`, and a branch hold most printable ASCII, which
//! an ASCII cell refuses. A branch with `|` or a backtick (git takes both)
//! no code cell holds, so it is refused when written.
use super::cells;
use super::{ArtifactCodec, CodecError};
use crate::domain::project_board::entities::change::{Change, ChangeFields};
use crate::domain::project_board::value_objects::code_ref::{BranchName, CommitId, RepoName};
use crate::domain::project_board::value_objects::schema_error::SchemaError;
use crate::infrastructure::persistence::project_board::markdown::{
    CellKind, Column, Document, Limits, Shape,
};

const TASK: &str = "Task";
const NUMBER: &str = "Number";
const REPOSITORY: &str = "Repository";
const BRANCH: &str = "Branch";
const PULL_REQUEST: &str = "Pull request";
const MERGE_COMMIT: &str = "Merge commit";
const LINKED_BY: &str = "Linked by";
const UNLINKED_BY: &str = "Unlinked by";
const UNLINKED_AT: &str = "Unlinked at";
const CREATED: &str = "Created";
const UPDATED: &str = "Updated";

const fn column(name: &'static str, cell: CellKind) -> Column<'static> {
    Column { name, cell }
}

const FIELDS: &[Column<'static>] = &[
    column(TASK, CellKind::Ascii),
    column(NUMBER, CellKind::Ascii),
    column(REPOSITORY, CellKind::Code),
    column(BRANCH, CellKind::Code),
    column(PULL_REQUEST, CellKind::Ascii),
    column(MERGE_COMMIT, CellKind::Ascii),
    column(LINKED_BY, CellKind::Text),
    column(UNLINKED_BY, CellKind::Text),
    column(UNLINKED_AT, CellKind::Ascii),
    column(CREATED, CellKind::Ascii),
    column(UPDATED, CellKind::Ascii),
];

/// A change's file caps; the widest change the domain admits fits
/// (`the_widest_change_fits`).
pub const CHANGE_LIMITS: Limits = Limits {
    file_bytes: 8 * 1024,
    lines: 24,
    line_bytes: 2 * 1024,
};

pub struct ChangeCodec;

impl ArtifactCodec for ChangeCodec {
    type Value = Change;
    const KIND: &'static str = "change";
    const SCHEMA: u32 = 1;
    const SHAPE: Shape<'static> = Shape {
        kind: "change",
        fields: FIELDS,
        sections: &[],
        limits: CHANGE_LIMITS,
    };

    fn to_document(change: &Change) -> Result<Document, CodecError> {
        let change = change.fields();
        let mut fields = vec![
            (TASK, cells::slug(&change.task)),
            (NUMBER, cells::number(change.number.into())),
            (REPOSITORY, change.head_repo.as_str().to_string()),
            (BRANCH, change.branch.as_str().to_string()),
        ];
        if let Some(pr) = change.pr {
            fields.push((PULL_REQUEST, cells::reference(pr)));
        }
        if let Some(commit) = &change.merged {
            fields.push((MERGE_COMMIT, commit.as_str().to_string()));
        }
        fields.push((LINKED_BY, cells::identity(LINKED_BY, &change.linked_by)?));
        if let Some((by, at)) = &change.unlinked {
            fields.push((UNLINKED_BY, cells::identity(UNLINKED_BY, by)?));
            fields.push((UNLINKED_AT, cells::time(at)));
        }
        fields.push((CREATED, cells::time(&change.created)));
        fields.push((UPDATED, cells::time(&change.updated)));
        Ok(Document {
            schema: Self::SCHEMA,
            title: format!("Change {} of {}", change.number, change.task.as_str()),
            fields: fields
                .into_iter()
                .map(|(name, cell)| (name.into(), cell))
                .collect(),
            sections: vec![],
        })
    }

    fn from_document(document: &Document) -> Result<Change, CodecError> {
        let field = |name: &'static str| {
            document
                .fields
                .iter()
                .find(|(known, _)| known == name)
                .map(|(_, cell)| cell.as_str())
        };
        let required = |name: &'static str| field(name).ok_or_else(|| refused(name, "missing"));
        let schema = |name: &'static str| move |error: SchemaError| refused(name, error.problem);
        let unlinked = match (field(UNLINKED_BY), field(UNLINKED_AT)) {
            (Some(by), Some(at)) => Some((
                cells::read_identity(UNLINKED_BY, by)?,
                cells::read_time(UNLINKED_AT, at)?,
            )),
            (None, None) => None,
            _ => {
                return Err(refused(
                    UNLINKED_BY,
                    "both or neither of it and Unlinked at",
                ));
            }
        };
        let number = cells::read_number(NUMBER, required(NUMBER)?)?;
        let fields = ChangeFields {
            task: cells::read_slug(TASK, required(TASK)?)?,
            number: u32::try_from(number).map_err(|_| refused(NUMBER, "past u32"))?,
            head_repo: RepoName::parse(required(REPOSITORY)?).map_err(schema(REPOSITORY))?,
            branch: BranchName::parse(required(BRANCH)?).map_err(schema(BRANCH))?,
            pr: field(PULL_REQUEST)
                .map(|cell| cells::read_reference(PULL_REQUEST, cell))
                .transpose()?,
            merged: field(MERGE_COMMIT)
                .map(|cell| CommitId::parse(cell).map_err(schema(MERGE_COMMIT)))
                .transpose()?,
            linked_by: cells::read_identity(LINKED_BY, required(LINKED_BY)?)?,
            unlinked,
            created: cells::read_time(CREATED, required(CREATED)?)?,
            updated: cells::read_time(UPDATED, required(UPDATED)?)?,
        };
        Change::new(fields)
            .map_err(|error| refused("Change", format!("{}: {}", error.field, error.problem)))
    }
}

fn refused(field: &'static str, problem: impl Into<String>) -> CodecError {
    CodecError::Cell {
        field,
        problem: problem.into(),
    }
}

#[cfg(test)]
#[path = "change_tests.rs"]
mod tests;
