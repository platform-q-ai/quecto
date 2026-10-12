//! A review's file (`quecto-review`, epic #2481 M2c-2). Its title is
//! derived, `Review of change {n} of {task}`, and its State is its last
//! round's verdict: written for a reader, and read back only as the value
//! it must be (any other is not canonical). Its sections:
//!
//! - Rounds `[Round | Commit | Reviewer | At | Verdict]`, oldest first,
//!   numbered from 1;
//! - Findings `[Round | Finding | Severity | Location | Text]`, grouped by
//!   round in round order and numbered from 1 within each; a finding with
//!   no place has `-`. Only when some round has a finding.
//!
//! A place is a code cell: a path may hold what an ASCII cell would read
//! as a link or emphasis (`@`, `_`, `www.`).
use super::cells::{self, column, refused};
use super::{ArtifactCodec, CodecError};
use crate::domain::project_board::entities::review::{Finding, Review, ReviewFields, Round};
use crate::domain::project_board::value_objects::code_ref::CommitId;
use crate::domain::project_board::value_objects::vocabulary::{ReviewVerdict, Severity};
use crate::infrastructure::persistence::project_board::markdown::{
    Body, BodyShape, CellKind, Column, Document, Limits, Section, SectionShape, Shape,
};

const TASK: &str = "Task";
const CHANGE: &str = "Change";
const STATE: &str = "State";
const CREATED: &str = "Created";
const UPDATED: &str = "Updated";
const ROUNDS: &str = "Rounds";
const FINDINGS: &str = "Findings";

const FIELDS: &[Column<'static>] = &[
    column(TASK, CellKind::Ascii),
    column(CHANGE, CellKind::Ascii),
    column(STATE, CellKind::Ascii),
    column(CREATED, CellKind::Ascii),
    column(UPDATED, CellKind::Ascii),
];

const SECTIONS: &[SectionShape<'static>] = &[
    SectionShape {
        heading: ROUNDS,
        body: BodyShape::Table(&[
            column("Round", CellKind::Ascii),
            column("Commit", CellKind::Ascii),
            column("Reviewer", CellKind::Text),
            column("At", CellKind::Ascii),
            column("Verdict", CellKind::Ascii),
        ]),
    },
    SectionShape {
        heading: FINDINGS,
        body: BodyShape::Table(&[
            column("Round", CellKind::Ascii),
            column("Finding", CellKind::Ascii),
            column("Severity", CellKind::Ascii),
            column("Location", CellKind::Code),
            column("Text", CellKind::Text),
        ]),
    },
];

/// A review's file caps, from the widest review the domain admits
/// (`the_widest_review_fits`: 984,178 bytes, 8,274 lines, a 16,531-byte
/// line), with room to spare.
pub const REVIEW_LIMITS: Limits = Limits {
    file_bytes: 1024 * 1024,
    lines: 8_400,
    line_bytes: 20 * 1024,
};

pub struct ReviewCodec;

impl ArtifactCodec for ReviewCodec {
    type Value = Review;
    const SCHEMA: u32 = 1;
    const SHAPE: Shape<'static> = Shape {
        kind: "review",
        fields: FIELDS,
        sections: SECTIONS,
        limits: REVIEW_LIMITS,
    };

    fn to_document(review: &Review) -> Result<Document, CodecError> {
        let state = review.state();
        let review = review.fields();
        let fields = [
            (TASK, cells::slug(&review.task)),
            (CHANGE, cells::number(review.change.into())),
            (STATE, cells::word(state.as_str())),
            (CREATED, cells::time(&review.created)),
            (UPDATED, cells::time(&review.updated)),
        ];
        let mut rounds = vec![];
        let mut findings = vec![];
        for (index, round) in review.rounds.iter().enumerate() {
            let number = cells::number((index + 1) as u64);
            rounds.push(vec![
                number.clone(),
                round.commit.as_str().to_string(),
                cells::identity(ROUNDS, &round.reviewer)?,
                cells::time(&round.at),
                cells::word(round.verdict.as_str()),
            ]);
            for (place, finding) in round.findings.iter().enumerate() {
                findings.push(vec![
                    number.clone(),
                    cells::number((place + 1) as u64),
                    cells::word(finding.severity.as_str()),
                    cells::optional(FINDINGS, finding.location.clone())?,
                    finding.text.clone(),
                ]);
            }
        }
        let sections = [(ROUNDS, rounds), (FINDINGS, findings)]
            .into_iter()
            // A table only for what holds rows (no Findings is "none").
            .filter(|(_, rows)| matches!(rows.as_slice(), [_, ..]))
            .map(|(heading, rows)| Section {
                heading: heading.into(),
                body: Body::Table(rows),
            })
            .collect();
        Ok(Document {
            schema: Self::SCHEMA,
            title: format!(
                "Review of change {} of {}",
                review.change,
                review.task.as_str()
            ),
            fields: fields
                .into_iter()
                .map(|(name, cell)| (name.into(), cell))
                .collect(),
            sections,
        })
    }

    fn from_document(document: &Document) -> Result<Review, CodecError> {
        let fields = cells::Fields::of(document);
        let required = |name: &'static str| fields.required(name);
        // The State is read as a word; that it is the last verdict is the
        // canonical check's (`decode` writes the value again).
        cells::read_word(STATE, required(STATE)?, ReviewVerdict::parse)?;
        let mut rounds: Vec<Round> = vec![];
        for section in &document.sections {
            let Body::Table(rows) = &section.body else {
                return Err(refused("Review", "a table"));
            };
            for row in rows {
                match (section.heading.as_str(), row.as_slice()) {
                    (ROUNDS, [number, commit, reviewer, at, verdict]) => {
                        let expected = cells::number((rounds.len() + 1) as u64);
                        match *number == expected {
                            true => {}
                            false => {
                                return Err(refused(ROUNDS, "rounds numbered from 1, in order"));
                            }
                        }
                        rounds.push(Round {
                            commit: CommitId::parse(commit)
                                .map_err(|error| refused(ROUNDS, error.problem))?,
                            reviewer: cells::read_identity(ROUNDS, reviewer)?,
                            at: cells::read_time(ROUNDS, at)?,
                            verdict: cells::read_word(ROUNDS, verdict, ReviewVerdict::parse)?,
                            findings: vec![],
                        });
                    }
                    (FINDINGS, [round, number, severity, location, text]) => {
                        let finding = Finding {
                            location: cells::read_optional(location).map(str::to_string),
                            severity: cells::read_word(FINDINGS, severity, Severity::parse)?,
                            text: text.clone(),
                        };
                        next_finding(&mut rounds, round, number)?.push(finding);
                    }
                    (heading, _) => {
                        return Err(refused("Review", format!("a row of {heading:?}")));
                    }
                }
            }
        }
        // Every row read went to its round: the rounds and findings are as
        // many as the tables' rows.
        debug_assert_eq!(
            (
                rounds.len(),
                rounds
                    .iter()
                    .map(|round| round.findings.len())
                    .sum::<usize>()
            ),
            (table_rows(document, ROUNDS), table_rows(document, FINDINGS)),
            "every row placed"
        );
        let fields = ReviewFields {
            task: cells::read_slug(TASK, required(TASK)?)?,
            change: u32::try_from(cells::read_number(CHANGE, required(CHANGE)?)?)
                .map_err(|_| refused(CHANGE, "past u32"))?,
            rounds,
            created: cells::read_time(CREATED, required(CREATED)?)?,
            updated: cells::read_time(UPDATED, required(UPDATED)?)?,
        };
        Review::new(fields)
            .map_err(|error| refused("Review", format!("{}: {}", error.field, error.problem)))
    }
}

/// How many rows the table under `heading` holds (none without one).
fn table_rows(document: &Document, heading: &str) -> usize {
    document
        .sections
        .iter()
        .filter(|section| section.heading == heading)
        .map(|section| match &section.body {
            Body::Table(rows) => rows.len(),
            Body::Prose(_) => 0,
        })
        .sum()
}

/// The findings of the round `round` names, which the finding numbered
/// `number` comes next in: findings are grouped by round in round order,
/// numbered from 1 within each.
fn next_finding<'a>(
    rounds: &'a mut [Round],
    round: &str,
    number: &str,
) -> Result<&'a mut Vec<Finding>, CodecError> {
    let index = cells::read_number(FINDINGS, round)?;
    let number = cells::read_number(FINDINGS, number)?;
    // The last round with a finding so far: none comes from before it.
    let latest = rounds.iter().rposition(|round| !round.findings.is_empty());
    let at = usize::try_from(index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .filter(|at| *at < rounds.len() && latest.is_none_or(|latest| latest <= *at))
        .ok_or_else(|| refused(FINDINGS, "a round of the review's, in round order"))?;
    let findings = &mut rounds[at].findings;
    match number == (findings.len() + 1) as u64 {
        true => Ok(findings),
        false => Err(refused(FINDINGS, "findings numbered from 1 within a round")),
    }
}

#[cfg(test)]
#[path = "review_tests.rs"]
mod tests;
