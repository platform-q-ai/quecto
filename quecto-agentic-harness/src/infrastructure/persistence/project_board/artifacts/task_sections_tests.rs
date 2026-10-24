//! A task's description in its sections (#2705 T2) in the task's file: a
//! file section each, byte for byte; legacy text kept as its Description,
//! never rewritten; and a hand edit that mixes the two refused.
use super::super::{decode, encode};
use super::*;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::infrastructure::persistence::project_board::markdown::parse;
use crate::infrastructure::persistence::project_board::markdown::tests::oracle::outline_in;

const SECTIONED_TEXT: &str = "## Goal\nLand L12 (#2652): move the UDS presenters, mapping\nand \
     loop controllers into `interface/uds` roles.\n\n## Why\nPart of the layout epic \
     #2357.\n\n## Done when\n- [ ] Current master merged in, moves only\n- [x] CI green, \
     merged with a merge commit\n\n## Links\n- PR #2652 · issue #2370\n- Handoff: planning notes";

const FILE: &str = r#"<!-- quecto-task schema=1 -->
# Land L12: UDS presenters and mapping

| Field | Value |
|---|---|
| Id | land-l12 |
| Kind | task |
| Status | draft |
| Created | 2026-10-11T09:30:00Z |
| Updated | 2026-10-11T09:30:00Z |

## Goal
Land L12 (#2652): move the UDS presenters, mapping
and loop controllers into `interface/uds` roles.

## Why
Part of the layout epic #2357.

## Done when
| Done | Outcome |
|---|---|
| no | Current master merged in, moves only |
| yes | CI green, merged with a merge commit |

## Links
- PR #2652 · issue #2370
- Handoff: planning notes

## Depends on
| Task |
|---|
| l11 |
"#;

const LEGACY: &str = "Refresh PR #2652.\n## Goal\nnot a section: a legacy line";

fn fields(description: &str) -> TaskFields {
    let at = Timestamp::parse("2026-10-11T09:30:00Z").unwrap();
    TaskFields {
        id: Slug::parse("land-l12").unwrap(),
        title: "Land L12: UDS presenters and mapping".into(),
        kind: TaskKind::Task,
        description: description.into(),
        status: TaskStatus::Draft,
        parent: None,
        depends_on: vec![Slug::parse("l11").unwrap()],
        accepted_plan: None,
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at.clone(),
        updated: at,
    }
}

#[test]
fn a_sectioned_description_round_trips_byte_for_byte() {
    let task = Task::new(fields(SECTIONED_TEXT)).unwrap();
    let file = encode::<TaskCodec>(&task).unwrap();
    assert_eq!(file, FILE);
    let read = decode::<TaskCodec>(file.as_bytes()).unwrap();
    assert_eq!(read.fields().description, SECTIONED_TEXT);
    assert_eq!(read, task);
    assert_eq!(encode::<TaskCodec>(&read).unwrap(), FILE);
    let document = parse(FILE.as_bytes(), &TaskCodec::SHAPE).unwrap();
    assert_eq!(outline_in(FILE, &document, &TaskCodec::SHAPE), Ok(()));
}

#[test]
fn a_legacy_description_stays_its_description_never_rewritten() {
    let task = Task::new(fields(LEGACY)).unwrap();
    let file = encode::<TaskCodec>(&task).unwrap();
    assert!(
        file.contains("## Description\nRefresh PR #2652.\n\\## Goal\n"),
        "{file}"
    );
    let read = decode::<TaskCodec>(file.as_bytes()).unwrap();
    assert_eq!(read.fields().description, LEGACY);
    assert_eq!(encode::<TaskCodec>(&read).unwrap(), file);
}

#[test]
fn a_file_mixing_a_description_with_sections_or_misshaping_one_is_refused() {
    let refused = |from: &str, to: &str| {
        assert!(FILE.contains(from), "{from:?}");
        let edited = FILE.replacen(from, to, 1);
        decode::<TaskCodec>(edited.as_bytes()).expect_err(&edited)
    };
    let mixed = refused("## Goal\n", "## Description\nOld text.\n\n## Goal\n");
    assert_eq!(
        mixed,
        cells::refused(super::DESCRIPTION, "beside its own sections")
    );
    let not_a_word = refused("| yes |", "| maybe |");
    assert_eq!(
        not_a_word,
        cells::refused(DONE_WHEN, "a word of its vocabulary")
    );
    let not_a_link = refused("\n- Handoff", "\n\nHandoff");
    assert_eq!(
        not_a_link,
        cells::refused(super::DESCRIPTION, NOT_A_DESCRIPTION)
    );
    let goal = "## Goal\nLand L12 (#2652): move the UDS presenters, mapping\nand loop controllers \
                into `interface/uds` roles.\n\n";
    assert_eq!(
        refused(goal, ""),
        cells::refused(super::DESCRIPTION, NOT_A_DESCRIPTION)
    );
}

#[test]
fn an_outcome_a_cell_holds_round_trips() {
    let description = "## Goal\nShip.\n\n## Done when\n- [ ] A `code | pipe` and \\ backslash";
    let task = Task::new(fields(description)).unwrap();
    let file = encode::<TaskCodec>(&task).unwrap();
    assert_eq!(decode::<TaskCodec>(file.as_bytes()), Ok(task));
}
