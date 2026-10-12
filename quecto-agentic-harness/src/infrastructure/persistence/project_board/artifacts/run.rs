//! A run's file (`quecto-run`, epic #2481 M2c-1). Its title is derived,
//! `Run {number} of {task}`. Its State is `running` while the run goes,
//! else the outcome it folded back with; Ended and Cost tokens rows come
//! only with the fold-back, as do its sections, each in item order:
//!
//! - Items `[Item | State]`, every item the run reports;
//! - Owners `[Item | Owner]`, only the items that have one;
//! - Evidence `[Item | Evidence]`, one row per line, grouped by item.
use super::cells::{self, column, refused};
use super::{ArtifactCodec, CodecError};
use crate::domain::project_board::entities::run::{Run, RunFields, RunItem};
use crate::domain::project_board::value_objects::vocabulary::{ItemState, RunOutcome};
use crate::infrastructure::persistence::project_board::markdown::{
    Body, BodyShape, CellKind, Column, Document, Limits, Section, SectionShape, Shape,
};

const TASK: &str = "Task";
const NUMBER: &str = "Number";
const PLAN_VERSION: &str = "Plan version";
const STATE: &str = "State";
const STARTED: &str = "Started";
const ENDED: &str = "Ended";
const COST: &str = "Cost tokens";
const ITEMS: &str = "Items";
const OWNERS: &str = "Owners";
const EVIDENCE: &str = "Evidence";
/// The State of a run that has not folded back.
const RUNNING: &str = "running";

const FIELDS: &[Column<'static>] = &[
    column(TASK, CellKind::Ascii),
    column(NUMBER, CellKind::Ascii),
    column(PLAN_VERSION, CellKind::Ascii),
    column(STATE, CellKind::Ascii),
    column(STARTED, CellKind::Ascii),
    column(ENDED, CellKind::Ascii),
    column(COST, CellKind::Ascii),
];

const SECTIONS: &[SectionShape<'static>] = &[
    SectionShape {
        heading: ITEMS,
        body: BodyShape::Table(&[
            column("Item", CellKind::Ascii),
            column(STATE, CellKind::Ascii),
        ]),
    },
    SectionShape {
        heading: OWNERS,
        body: BodyShape::Table(&[
            column("Item", CellKind::Ascii),
            column("Owner", CellKind::Text),
        ]),
    },
    SectionShape {
        heading: EVIDENCE,
        body: BodyShape::Table(&[
            column("Item", CellKind::Ascii),
            column(EVIDENCE, CellKind::Text),
        ]),
    },
];

/// A run's file caps; the widest run the domain admits fits
/// (`the_widest_run_fits`).
pub const RUN_LIMITS: Limits = Limits {
    file_bytes: 768 * 1024,
    lines: 2_400,
    line_bytes: 8 * 1024,
};

pub struct RunCodec;

impl ArtifactCodec for RunCodec {
    type Value = Run;
    const SCHEMA: u32 = 1;
    const SHAPE: Shape<'static> = Shape {
        kind: "run",
        fields: FIELDS,
        sections: SECTIONS,
        limits: RUN_LIMITS,
    };

    fn to_document(run: &Run) -> Result<Document, CodecError> {
        let run = run.fields();
        // What the State and Ended rows rest on: an outcome exactly when
        // the run ended, and none spelt `running`.
        debug_assert_eq!(run.outcome.is_some(), run.ended.is_some());
        debug_assert!(
            run.outcome
                .is_none_or(|outcome| outcome.as_str() != RUNNING)
        );
        let state = run.outcome.map_or(RUNNING, RunOutcome::as_str);
        let mut fields = vec![
            (TASK, cells::slug(&run.task)),
            (NUMBER, cells::number(run.number.into())),
            (PLAN_VERSION, cells::number(run.plan_version.into())),
            (STATE, cells::word(state)),
            (STARTED, cells::time(&run.started)),
        ];
        if let Some(ended) = &run.ended {
            fields.push((ENDED, cells::time(ended)));
        }
        if let Some(cost) = run.cost {
            fields.push((COST, cells::number(cost)));
        }
        let id = |item: &RunItem| cells::slug(&item.id);
        let items = run
            .items
            .iter()
            .map(|item| vec![id(item), cells::word(item.state.as_str())]);
        let owners = run.items.iter().filter_map(|item| {
            let owner = item.owner.as_ref()?;
            Some(vec![id(item), owner.clone()])
        });
        let evidence = run.items.iter().flat_map(|item| {
            item.evidence
                .iter()
                .map(move |line| vec![id(item), line.clone()])
        });
        let sections = [
            (ITEMS, items.collect::<Vec<_>>()),
            (OWNERS, owners.collect()),
            (EVIDENCE, evidence.collect()),
        ]
        .into_iter()
        // A table only for what holds rows (a missing one is "none").
        .filter(|(_, rows)| matches!(rows.as_slice(), [_, ..]))
        .map(|(heading, rows)| Section {
            heading: heading.into(),
            body: Body::Table(rows),
        })
        .collect();
        Ok(Document {
            schema: Self::SCHEMA,
            title: format!("Run {} of {}", run.number, run.task.as_str()),
            fields: fields
                .into_iter()
                .map(|(name, cell)| (name.into(), cell))
                .collect(),
            sections,
        })
    }

    fn from_document(document: &Document) -> Result<Run, CodecError> {
        let fields = cells::Fields::of(document);
        let field = |name: &str| fields.get(name);
        let required = |name: &'static str| fields.required(name);
        let outcome = match required(STATE)? {
            RUNNING => None,
            word => Some(cells::read_word(STATE, word, RunOutcome::parse)?),
        };
        let ended = field(ENDED)
            .map(|cell| cells::read_time(ENDED, cell))
            .transpose()?;
        match (outcome, &ended) {
            (None, None) | (Some(_), Some(_)) => {}
            (None, Some(_)) => return Err(refused(ENDED, "a running run has not ended")),
            (Some(_), None) => return Err(refused(ENDED, "missing for a run that ended")),
        }
        let mut items: Vec<RunItem> = vec![];
        for section in &document.sections {
            let Body::Table(rows) = &section.body else {
                return Err(refused("Run", "a table"));
            };
            for row in rows {
                let [id, cell] = row.as_slice() else {
                    return Err(refused("Run", "a row of two cells"));
                };
                match section.heading.as_str() {
                    ITEMS => items.push(RunItem {
                        id: cells::read_slug(ITEMS, id)?,
                        state: cells::read_word(ITEMS, cell, ItemState::parse)?,
                        owner: None,
                        evidence: vec![],
                    }),
                    OWNERS => match &mut item(&mut items, OWNERS, id)?.owner {
                        owner @ None => *owner = Some(cell.clone()),
                        Some(_) => return Err(refused(OWNERS, "one owner an item")),
                    },
                    EVIDENCE => item(&mut items, EVIDENCE, id)?.evidence.push(cell.clone()),
                    other => return Err(refused("Run", format!("a section {other:?}"))),
                }
            }
        }
        let fields = RunFields {
            task: cells::read_slug(TASK, required(TASK)?)?,
            number: narrow(NUMBER, cells::read_number(NUMBER, required(NUMBER)?)?)?,
            plan_version: narrow(
                PLAN_VERSION,
                cells::read_number(PLAN_VERSION, required(PLAN_VERSION)?)?,
            )?,
            started: cells::read_time(STARTED, required(STARTED)?)?,
            ended,
            outcome,
            cost: field(COST)
                .map(|cell| cells::read_number(COST, cell))
                .transpose()?,
            items,
        };
        Run::new(fields)
            .map_err(|error| refused("Run", format!("{}: {}", error.field, error.problem)))
    }
}

/// The item `id` names among those read so far.
fn item<'a>(
    items: &'a mut [RunItem],
    table: &'static str,
    id: &str,
) -> Result<&'a mut RunItem, CodecError> {
    let id = cells::read_slug(table, id)?;
    items
        .iter_mut()
        .find(|item| item.id == id)
        .ok_or_else(|| refused(table, format!("{} is no item of the run", id.as_str())))
}

fn narrow(field: &'static str, number: u64) -> Result<u32, CodecError> {
    u32::try_from(number).map_err(|_| refused(field, "past u32"))
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
