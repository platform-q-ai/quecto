//! A plan's handoff in its file (#2705 T3): the schema marker, never the
//! text, picks the layout. A plan with a handoff is written at schema 2
//! with a `## Handoff` section after its approach; any other stays at
//! schema 1, the bytes master wrote, so master's files read, re-encode
//! and, edited without a handoff, stay schema 1 byte for byte.
use super::super::{CodecError, decode, encode};
use super::*;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::plan::{MAX_APPROACH_CHARS, MAX_HANDOFF_CHARS};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// A draft master (before T3) wrote, its approach far past the new cap.
const MASTER_LONG: &str = r#"<!-- quecto-plan schema=1 -->
# Plan 1 of board-cli

| Field | Value |
|---|---|
| Task | board-cli |
| Version | 1 |
| State | draft |
| Proposed by | Ada (`ada@example.com`) |
| Created | 2026-10-11T09:30:00Z |
| Updated | 2026-10-11T09:30:00Z |

## Approach
Build the store first, then the CLI on it, one op a line. Build the store first, then the CLI on it, one op a line. Build the store first, then the CLI on it, one op a line. Build the store first, then the CLI on it, one op a line. Build the store first, then the CLI on it, one op a line. Build the store first, then the CLI on it, one op a line. Build the store first, then the CLI on it, one op a line.

Build the store first, then the CLI on it, one op a line. Build the store first, then the CLI on it, one op a line. Build the store first, then the CLI on it, one op a line. Build the store first, then the CLI on it, one op a line. Build the store first, then the CLI on it, one op a line. Build the store first, then the CLI on it, one op a line. Build the store first, then the CLI on it, one op a line.

## Task items
| Task item | Title |
|---|---|
| store | The store |

## Criteria
| Task item | Criterion |
|---|---|
| store | Round trips |
"#;

/// An accepted plan master wrote, its approach holding lines that look
/// like a `## Handoff` heading (written escaped, as prose).
const MASTER_HEADING: &str = r#"<!-- quecto-plan schema=1 -->
# Plan 2 of board-cli

| Field | Value |
|---|---|
| Task | board-cli |
| Version | 2 |
| State | accepted |
| Proposed by | Ada (`ada@example.com`) |
| Edited by | Bo (`bo@example.com`) |
| Accepted by | Bo (`bo@example.com`) |
| Accepted at | 2026-10-11T12:00:00Z |
| Created | 2026-10-11T09:30:00Z |
| Updated | 2026-10-11T12:00:00Z |

## Approach
Outline the store, then the CLI.
\## Handoff
Not a section: a line of the approach.

\## Task items
Still the approach.

## Task items
| Task item | Title |
|---|---|
| store | The store |

## Criteria
| Task item | Criterion |
|---|---|
| store | Round trips |
"#;

/// A plan with a handoff, as this build writes it.
const WITH_HANDOFF: &str = r#"<!-- quecto-plan schema=2 -->
# Plan 3 of board-cli

| Field | Value |
|---|---|
| Task | board-cli |
| Version | 3 |
| State | draft |
| Proposed by | Ada (`ada@example.com`) |
| Created | 2026-10-11T09:30:00Z |
| Updated | 2026-10-11T09:30:00Z |

## Approach
Outline: the store, then the CLI.

## Handoff
Scope: the store only.
\## Not a section

- run `cargo test`

## Task items
| Task item | Title |
|---|---|
| store | The store |

## Criteria
| Task item | Criterion |
|---|---|
| store | Round trips |
"#;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn at(text: &str) -> Timestamp {
    Timestamp::parse(text).unwrap()
}

fn person(name: &str) -> Identity {
    Identity {
        name: name.into(),
        email: format!("{}@example.com", name.to_lowercase()),
    }
}

fn store_item() -> PlanItem {
    PlanItem {
        id: slug("store"),
        title: "The store".into(),
        criteria: vec!["Round trips".into()],
        depends_on: vec![],
    }
}

fn long_approach() -> String {
    let paragraph = "Build the store first, then the CLI on it, one op a line. ".repeat(7);
    format!("{}\n\n{}", paragraph.trim_end(), paragraph.trim_end())
}

fn master_long() -> PlanFields {
    PlanFields {
        task: slug("board-cli"),
        version: 1,
        state: PlanState::Draft,
        approach: long_approach(),
        handoff: None,
        items: vec![store_item()],
        team: None,
        proposed_by: person("Ada"),
        edited_by: None,
        accepted: None,
        created: at("2026-10-11T09:30:00Z"),
        updated: at("2026-10-11T09:30:00Z"),
    }
}

fn master_heading() -> PlanFields {
    PlanFields {
        version: 2,
        state: PlanState::Accepted,
        approach: "Outline the store, then the CLI.\n## Handoff\nNot a section: a line of \
                   the approach.\n\n## Task items\nStill the approach."
            .into(),
        edited_by: Some(person("Bo")),
        accepted: Some((person("Bo"), at("2026-10-11T12:00:00Z"))),
        updated: at("2026-10-11T12:00:00Z"),
        ..master_long()
    }
}

fn with_handoff() -> PlanFields {
    PlanFields {
        version: 3,
        approach: "Outline: the store, then the CLI.".into(),
        handoff: Some("Scope: the store only.\n## Not a section\n\n- run `cargo test`".into()),
        ..master_long()
    }
}

#[test]
fn master_plan_files_read_and_write_byte_for_byte() {
    assert!(long_approach().chars().count() > 600);
    for (file, fields) in [
        (MASTER_LONG, master_long()),
        (MASTER_HEADING, master_heading()),
    ] {
        let plan = Plan::new(fields).unwrap();
        assert_eq!(decode::<PlanCodec>(file.as_bytes()), Ok(plan.clone()));
        assert_eq!(
            plan.fields().handoff,
            None,
            "a heading in prose is no handoff"
        );
        assert_eq!(encode::<PlanCodec>(&plan).unwrap(), file);
    }
}

/// A master file edited as a plan is without a handoff (here accepted,
/// then superseded) stays at schema 1, each line but the ones the edit
/// changed as master wrote it.
#[test]
fn a_master_plan_edited_without_a_handoff_stays_at_schema_1() {
    let read = decode::<PlanCodec>(MASTER_HEADING.as_bytes()).unwrap();
    let mut superseded = read.into_fields();
    superseded.state = PlanState::Superseded;
    let file = encode::<PlanCodec>(&Plan::new(superseded).unwrap()).unwrap();
    assert_eq!(
        file,
        MASTER_HEADING.replace("| State | accepted |", "| State | superseded |")
    );
}

#[test]
fn a_plan_with_a_handoff_is_written_at_schema_2() {
    let plan = Plan::new(with_handoff()).unwrap();
    assert_eq!(encode::<PlanCodec>(&plan).unwrap(), WITH_HANDOFF);
    assert_eq!(
        decode::<PlanCodec>(WITH_HANDOFF.as_bytes()),
        Ok(plan.clone())
    );
    // Taken away, it goes back to schema 1.
    let mut fields = plan.into_fields();
    fields.handoff = None;
    let file = encode::<PlanCodec>(&Plan::new(fields).unwrap()).unwrap();
    assert!(
        file.starts_with("<!-- quecto-plan schema=1 -->\n"),
        "{file}"
    );
    assert!(!file.contains("## Handoff"), "{file}");
}

/// The marker, never the text, picks the layout: a Handoff section in a
/// schema-1 file (no build wrote one) and a schema-2 file without one are
/// refused, never read as something near them.
#[test]
fn a_handoff_is_read_only_at_schema_2() {
    let at_schema_1 = WITH_HANDOFF.replace("schema=2", "schema=1");
    let refused = decode::<PlanCodec>(at_schema_1.as_bytes()).unwrap_err();
    assert!(
        matches!(
            refused,
            CodecError::Cell {
                field: "Handoff",
                ..
            }
        ),
        "{refused:?}"
    );
    let without = MASTER_LONG.replace("schema=1", "schema=2");
    let refused = decode::<PlanCodec>(without.as_bytes()).unwrap_err();
    assert!(
        matches!(
            refused,
            CodecError::Cell {
                field: "Handoff",
                ..
            }
        ),
        "{refused:?}"
    );
    // A schema past this build's is a newer quecto's: an older build
    // (which reads schema 1 alone) answers a schema-2 plan the same way.
    let newer = WITH_HANDOFF.replace("schema=2", "schema=3");
    assert_eq!(
        decode::<PlanCodec>(newer.as_bytes()),
        Err(CodecError::NewerSchema { found: 3, known: 2 })
    );
}

/// The widest plan with a handoff still fits the file's caps: an approach
/// and a handoff each at their most, all blank lines inside fences.
#[test]
fn the_longest_approach_and_handoff_fit_the_file() {
    let fenced = |chars: usize| format!("```{}```", "\n".repeat(chars - 6));
    let fields = PlanFields {
        approach: fenced(MAX_APPROACH_CHARS),
        handoff: Some(fenced(MAX_HANDOFF_CHARS)),
        ..with_handoff()
    };
    let plan = Plan::new(fields).unwrap();
    let file = encode::<PlanCodec>(&plan).unwrap();
    assert!(
        file.lines().count() > 2 * 65_000,
        "{}",
        file.lines().count()
    );
    assert_eq!(decode::<PlanCodec>(file.as_bytes()), Ok(plan));
}
