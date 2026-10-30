//! A task's description in its sections (#2705 T2) in the task's file:
//! its form, never its text's shape, picks the layout. Kept in its
//! sections it is a file section each, at schema 2, byte for byte; legacy
//! text stays its Description at schema 1, the bytes master wrote, however
//! it reads; and a hand edit that mixes the two is refused.
use super::super::{CodecError, decode, encode};
use super::*;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::infrastructure::persistence::project_board::markdown::parse;
use crate::infrastructure::persistence::project_board::markdown::tests::oracle::outline_in;

const SECTIONED_TEXT: &str = "## Goal\nLand L12 (#2652): move the UDS presenters, mapping\nand \
     loop controllers into `interface/uds` roles.\n\n## Why\nPart of the layout epic \
     #2357.\n\n## Done when\n- [ ] Current master merged in, moves only\n- [x] CI green, \
     merged with a merge commit\n\n## Links\n- [PR #2652](https://github.com/o/r/pull/2652)\n- \
     [Handoff](https://example.com/handoff)";

const FILE: &str = r#"<!-- quecto-task schema=2 -->
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
- [PR #2652](https://github.com/o/r/pull/2652)
- [Handoff](https://example.com/handoff)

## Depends on
| Task |
|---|
| l11 |
"#;

/// Files master wrote (schema 1), legacy text in a Description: one in
/// the sections' very shape, escaped as master escapes it, and one free.
const MASTER_FILES: [&str; 2] = [
    r#"<!-- quecto-task schema=1 -->
# Ship it

| Field | Value |
|---|---|
| Id | ship |
| Kind | task |
| Status | draft |
| Created | 2026-10-11T09:30:00Z |
| Updated | 2026-10-11T09:30:00Z |

## Description
\## Goal
Ship it.

\## Done when
\- [ ] Merged
"#,
    r#"<!-- quecto-task schema=1 -->
# Ship it

| Field | Value |
|---|---|
| Id | ship |
| Kind | task |
| Status | ready |
| Created | 2026-10-11T09:30:00Z |
| Updated | 2026-10-11T09:30:00Z |

## Description
Refresh PR #2652 after #2641; moves only.
"#,
];

const LEGACY: &str = "Refresh PR #2652.\n## Goal\nnot a section: a legacy line";

fn fields(description: &str, form: DescriptionForm) -> TaskFields {
    let at = Timestamp::parse("2026-10-11T09:30:00Z").unwrap();
    TaskFields {
        id: Slug::parse("land-l12").unwrap(),
        title: "Land L12: UDS presenters and mapping".into(),
        kind: TaskKind::Task,
        description: description.into(),
        description_form: form,
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
        reason: None,
    }
}

#[test]
fn a_sectioned_description_round_trips_byte_for_byte_at_schema_2() {
    let task = Task::new(fields(SECTIONED_TEXT, DescriptionForm::Sectioned)).unwrap();
    let file = encode::<TaskCodec>(&task).unwrap();
    assert_eq!(file, FILE);
    let read = decode::<TaskCodec>(file.as_bytes()).unwrap();
    assert_eq!(read.fields().description, SECTIONED_TEXT);
    assert_eq!(read, task);
    assert_eq!(encode::<TaskCodec>(&read).unwrap(), FILE);
    let document = parse(FILE.as_bytes(), &TaskCodec::SHAPE).unwrap();
    assert_eq!(outline_in(FILE, &document, &TaskCodec::SHAPE), Ok(()));
}

/// Review round 1 (HIGH): a file master wrote reads and writes again to
/// the very same bytes, one whose legacy text is in the sections' shape
/// included: the form, kept as legacy, picks the layout, never the text.
#[test]
fn master_written_files_decode_and_encode_byte_for_byte() {
    for file in MASTER_FILES {
        let task = decode::<TaskCodec>(file.as_bytes()).unwrap();
        assert_eq!(task.fields().description_form, DescriptionForm::Legacy);
        assert_eq!(encode::<TaskCodec>(&task).unwrap(), file);
    }
    let shaped = decode::<TaskCodec>(MASTER_FILES[0].as_bytes()).unwrap();
    assert_eq!(
        shaped.fields().description,
        "## Goal\nShip it.\n\n## Done when\n- [ ] Merged"
    );
}

#[test]
fn a_legacy_description_stays_its_description_never_rewritten() {
    for text in [LEGACY, SECTIONED_TEXT] {
        let task = Task::new(fields(text, DescriptionForm::Legacy)).unwrap();
        let file = encode::<TaskCodec>(&task).unwrap();
        assert!(
            file.starts_with("<!-- quecto-task schema=1 -->\n"),
            "{file}"
        );
        assert!(file.contains("\n## Description\n"), "{file}");
        let read = decode::<TaskCodec>(file.as_bytes()).unwrap();
        assert_eq!(read, task);
        assert_eq!(encode::<TaskCodec>(&read).unwrap(), file);
    }
}

/// Review round 1 (MEDIUM): a file of a newer schema asks for an upgrade,
/// as master asks of schema 2 (it reads schema 1 alone); schema 3 holds a
/// reason since #2705 T4.
#[test]
fn a_newer_schema_asks_for_an_upgrade() {
    let newer = FILE.replacen("schema=2", "schema=5", 1);
    assert_eq!(
        decode::<TaskCodec>(newer.as_bytes()),
        Err(CodecError::NewerSchema { found: 5, known: 4 })
    );
}

#[test]
fn a_file_mixing_the_layouts_or_misshaping_one_is_refused() {
    let refused = |file: &str, from: &str, to: &str| {
        assert!(file.contains(from), "{from:?}");
        let edited = file.replacen(from, to, 1);
        decode::<TaskCodec>(edited.as_bytes()).expect_err(&edited)
    };
    let mixed = refused(FILE, "## Goal\n", "## Description\nOld text.\n\n## Goal\n");
    assert_eq!(mixed, cells::refused(DESCRIPTION, BESIDE_SECTIONS));
    let at_one = refused(FILE, "schema=2", "schema=1");
    assert_eq!(at_one, cells::refused(DESCRIPTION, SECTIONS_AT_SCHEMA_1));
    let not_a_word = refused(FILE, "| yes |", "| maybe |");
    assert_eq!(
        not_a_word,
        cells::refused(DONE_WHEN, "a word of its vocabulary")
    );
    let not_a_link = refused(FILE, "\n- [Handoff]", "\n\n[Handoff]");
    assert_eq!(not_a_link, cells::refused(DESCRIPTION, NOT_A_DESCRIPTION));
    let goal = "## Goal\nLand L12 (#2652): move the UDS presenters, mapping\nand loop controllers \
                into `interface/uds` roles.\n\n";
    assert_eq!(
        refused(FILE, goal, ""),
        cells::refused(DESCRIPTION, NOT_A_DESCRIPTION)
    );
    let legacy_at_two = refused(MASTER_FILES[1], "schema=1", "schema=2");
    assert_eq!(legacy_at_two, cells::refused(DESCRIPTION, BESIDE_SECTIONS));
}

#[test]
fn awkward_sections_round_trip() {
    for description in [
        "## Goal\nShip.\n\n## Done when\n- [ ] A `code | pipe` and \\ backslash",
        "## Goal\nLand it.\n#2652 goes first.\n\n## Done when\n- [ ] Done\n\n## Links\n- \
         [#2652](https://github.com/o/r/pull/2652)",
        "## Goal\nShip.\n#######\n\n## Why\n- a list\n- of reasons\n\n## Done when\n- [x] Done",
    ] {
        let task = Task::new(fields(description, DescriptionForm::Sectioned)).unwrap();
        let file = encode::<TaskCodec>(&task).unwrap();
        assert_eq!(decode::<TaskCodec>(file.as_bytes()), Ok(task), "{file}");
    }
}

#[test]
fn done_ticks_old_done_file_reads_as_met_and_round_trips() {
    // Simulate an older file directly, not Task::new: compatibility belongs
    // at the read boundary and must not depend on constructor normalization.
    let old = FILE.replace(
        "| Status | draft |",
        "| Status | done |\n| Resolution | completed |",
    );
    let read = decode::<TaskCodec>(old.as_bytes()).unwrap();
    assert_eq!(read.fields().status, TaskStatus::Done);
    assert_eq!(
        read.fields().description,
        SECTIONED_TEXT.replace("- [ ]", "- [x]")
    );
    let written = encode::<TaskCodec>(&read).unwrap();
    assert_eq!(decode::<TaskCodec>(written.as_bytes()).unwrap(), read);
    assert!(old.contains("| no | Current master merged in, moves only |"));
}

#[test]
fn done_ticks_unfinished_files_keep_unchecked_outcomes() {
    for status in ["draft", "ready", "review", "blocked"] {
        let file = FILE.replace("| Status | draft |", &format!("| Status | {status} |"));
        let read = decode::<TaskCodec>(file.as_bytes()).unwrap();
        assert_eq!(read.fields().description, SECTIONED_TEXT, "{status}");
        assert_eq!(encode::<TaskCodec>(&read).unwrap(), file, "{status}");
    }
}

#[test]
fn done_ticks_legacy_and_unrelated_archived_text_stays_unchanged() {
    for status in [TaskStatus::Done, TaskStatus::Archived] {
        let mut legacy = fields(SECTIONED_TEXT, DescriptionForm::Legacy);
        legacy.status = status;
        legacy.resolution = match status {
            TaskStatus::Done => {
                Some(crate::domain::project_board::value_objects::vocabulary::Resolution::Completed)
            }
            _ => None,
        };
        let task = Task::new(legacy).unwrap();
        let file = encode::<TaskCodec>(&task).unwrap();
        assert_eq!(decode::<TaskCodec>(file.as_bytes()).unwrap(), task);
    }
    let archived = FILE.replace("| Status | draft |", "| Status | archived |");
    let read = decode::<TaskCodec>(archived.as_bytes()).unwrap();
    assert_eq!(read.fields().description, SECTIONED_TEXT);
    assert_eq!(encode::<TaskCodec>(&read).unwrap(), archived);
}

/// Parent approved explicit exception metadata; schema 4 is next after 3.
/// Mechanical encoding uses the existing identity cell and timestamp spelling.
const DONE_TICKS_EXCEPTION_SECTION: &str = "## Outcome exceptions\n| Outcome index | Reason | By | At |\n|---|---|---|---|\n| 0 | Owner deferred deployment | Ada (`ada@example.com`) | 2026-10-11T09:30:00Z |\n\n";

fn done_ticks_exception_file(status: &str) -> String {
    FILE.replace("schema=2", "schema=4")
        .replace("| Status | draft |", &format!("| Status | {status} |\n| Resolution | done_without_run |\n| Reason | Answered by hand |"))
        .replace("## Links\n", &format!("{DONE_TICKS_EXCEPTION_SECTION}## Links\n"))
}

#[test]
fn done_ticks_schema4_exception_survives_done_and_archive_codec_roundtrips() {
    for status in ["done", "archived"] {
        let file = done_ticks_exception_file(status);
        let read =
            decode::<TaskCodec>(file.as_bytes()).expect("approved schema4 exception is readable");
        assert_eq!(read.fields().reason.as_deref(), Some("Answered by hand"));
        let expected = SECTIONED_TEXT.replace(
            "## Links\n",
            &format!("{DONE_TICKS_EXCEPTION_SECTION}## Links\n"),
        );
        assert_eq!(
            read.fields().description,
            expected,
            "explicit unmet is not implicit met"
        );
        let parsed = Description::read(&read.fields().description)
            .expect("exception text remains parseable");
        assert!(!parsed.done_when()[0].done);
        assert!(parsed.done_when()[1].done);
        assert_eq!(
            parsed.done_when()[0].text,
            "Current master merged in, moves only"
        );
        assert_eq!(parsed.markdown(), expected);
        let encoded = encode::<TaskCodec>(&read).unwrap();
        assert_eq!(encoded, file, "metadata, reasons and attribution preserved");
        assert_eq!(decode::<TaskCodec>(encoded.as_bytes()).unwrap(), read);
    }
}

#[test]
fn done_ticks_exception_description_is_parseable_without_changing_outcome_text() {
    let description = SECTIONED_TEXT.replace(
        "## Links\n",
        &format!("{DONE_TICKS_EXCEPTION_SECTION}## Links\n"),
    );
    let parsed = Description::read(&description).expect("approved exception description grammar");
    assert_eq!(parsed.markdown(), description);
    assert_eq!(
        parsed.done_when()[0].text,
        "Current master merged in, moves only"
    );
    assert!(!parsed.done_when()[0].done);
}

#[test]
fn done_ticks_exception_codec_refuses_inconsistent_or_unattributed_metadata() {
    let file = done_ticks_exception_file("done");
    for malformed in [
        file.replace("| 0 | Owner", "| 2 | Owner"),
        file.replace("| 0 | Owner", "| 1 | Owner"),
        file.replace("| 0 | Owner", "| 00 | Owner"),
        file.replace("Owner deferred deployment", " "),
        file.replace("Ada (`ada@example.com`)", "Ada"),
        file.replace("2026-10-11T09:30:00Z", "not-a-time"),
        file.replace("schema=4", "schema=3"),
        file.replace("| 0 | Owner deferred deployment | Ada (`ada@example.com`) | 2026-10-11T09:30:00Z |", "| 0 | Owner deferred deployment | Ada (`ada@example.com`) | 2026-10-11T09:30:00Z |\n| 0 | Owner deferred deployment | Ada (`ada@example.com`) | 2026-10-11T09:30:00Z |"),
    ] {
        assert!(decode::<TaskCodec>(malformed.as_bytes()).is_err(), "{malformed}");
    }
}

#[test]
fn done_ticks_exception_text_cells_escape_and_roundtrip_without_changing_meaning() {
    let read = decode::<TaskCodec>(done_ticks_exception_file("done").as_bytes()).unwrap();
    let by = crate::domain::project_board::entities::claim::Identity {
        name: "Ada | Ops".into(),
        email: "a|b@example.com".into(),
    };
    for reason in [
        r"Owner | deferred\deployment",
        r"See `a\|b`",
        r"See ``a`\|b``",
        r"Literal \* marker",
        "Owner `deferred`",
        "Owner **deferred** deployment",
        "See [decision](https://example.com/decision)",
        "Contact `owner@example.com`",
    ] {
        let task = read
            .set_outcome(0, Some(reason), &by, &read.fields().updated)
            .unwrap();
        let encoded = encode::<TaskCodec>(&task).unwrap();
        assert_eq!(
            decode::<TaskCodec>(encoded.as_bytes()).unwrap(),
            task,
            "{encoded}"
        );
        let description = Description::read(&task.fields().description).unwrap();
        assert_eq!(
            description.done_when()[0]
                .exception
                .as_ref()
                .unwrap()
                .reason,
            reason
        );
    }
}

#[test]
fn done_ticks_schema4_settles_only_implicit_done_outcomes() {
    let file = done_ticks_exception_file("done").replace("| yes |", "| no |");
    let task = decode::<TaskCodec>(file.as_bytes()).unwrap();
    let description = Description::read(&task.fields().description).unwrap();
    assert!(!description.done_when()[0].done);
    assert!(description.done_when()[0].exception.is_some());
    assert!(description.done_when()[1].done);
    assert!(description.done_when()[1].exception.is_none());
    assert_eq!(
        decode::<TaskCodec>(encode::<TaskCodec>(&task).unwrap().as_bytes()).unwrap(),
        task
    );
}

#[test]
fn done_ticks_exception_metadata_cannot_enter_as_a_general_description_edit() {
    let text = SECTIONED_TEXT.replace(
        "## Links\n",
        &format!("{DONE_TICKS_EXCEPTION_SECTION}## Links\n"),
    );
    assert!(Description::read(&text).is_some());
    assert!(
        Description::entered(&text).is_err(),
        "writer attribution is not caller-supplied description data"
    );
}

#[test]
fn done_ticks_archived_completed_files_settle_only_with_completion_evidence() {
    for resolution in ["completed", "done_without_run"] {
        let reason = match resolution {
            "done_without_run" => "\n| Reason | Answered by hand |",
            _ => "",
        };
        let archived = FILE.replace(
            "| Status | draft |",
            &format!("| Status | archived |\n| Resolution | {resolution} |{reason}"),
        );
        let archived = match resolution {
            "done_without_run" => archived.replace("schema=2", "schema=3"),
            _ => archived,
        };
        let read = decode::<TaskCodec>(archived.as_bytes()).unwrap();
        assert_eq!(
            read.fields().description,
            SECTIONED_TEXT.replace("- [ ]", "- [x]"),
            "formerly completed: {resolution}"
        );
        assert_eq!(
            decode::<TaskCodec>(encode::<TaskCodec>(&read).unwrap().as_bytes()).unwrap(),
            read
        );
    }
    for resolution in ["", "\n| Resolution | not_planned |"] {
        let archived = FILE.replace(
            "| Status | draft |",
            &format!("| Status | archived |{resolution}"),
        );
        let read = decode::<TaskCodec>(archived.as_bytes()).unwrap();
        assert_eq!(
            read.fields().description,
            SECTIONED_TEXT,
            "no completed history means no inference"
        );
    }
}

#[test]
fn done_ticks_domain_exception_validation_allows_only_done_and_archived() {
    for status in [
        TaskStatus::Draft,
        TaskStatus::Ready,
        TaskStatus::Claimed,
        TaskStatus::InProgress,
        TaskStatus::Review,
        TaskStatus::Blocked,
        TaskStatus::Done,
        TaskStatus::Archived,
    ] {
        let read = decode::<TaskCodec>(done_ticks_exception_file("done").as_bytes()).unwrap();
        let mut fields = read.into_fields();
        fields.status = status;
        match status {
            TaskStatus::Done | TaskStatus::Archived => {}
            _ => {
                fields.resolution = None;
                fields.reason = None;
            }
        }
        if matches!(status, TaskStatus::Claimed | TaskStatus::InProgress) {
            fields.claim = Some(crate::domain::project_board::entities::claim::Claim {
                holder: crate::domain::project_board::entities::claim::Identity {
                    name: "Ada".into(),
                    email: "ada@example.com".into(),
                },
                since: fields.updated.clone(),
                expires: fields.updated.plus_seconds(3600).unwrap(),
            });
        }
        let result = Task::new(fields);
        match status {
            TaskStatus::Done | TaskStatus::Archived => {
                assert!(result.is_ok(), "{status:?}: {result:?}")
            }
            _ => assert!(result.is_err(), "exception must not inhabit {status:?}"),
        }
    }
}

#[test]
fn done_ticks_schema4_is_emitted_only_while_an_exception_exists() {
    let read = decode::<TaskCodec>(done_ticks_exception_file("done").as_bytes()).unwrap();
    assert!(
        encode::<TaskCodec>(&read)
            .unwrap()
            .starts_with("<!-- quecto-task schema=4 -->")
    );
    let restored = read
        .set_outcome(
            0,
            None,
            &crate::domain::project_board::entities::claim::Identity {
                name: "Ada".into(),
                email: "ada@example.com".into(),
            },
            &read.fields().updated,
        )
        .unwrap();
    let encoded = encode::<TaskCodec>(&restored).unwrap();
    assert!(
        encoded.starts_with("<!-- quecto-task schema=3 -->"),
        "reason alone uses schema3: {encoded}"
    );
    assert_eq!(decode::<TaskCodec>(encoded.as_bytes()).unwrap(), restored);
}
