//! A status move's reason (#2705 T4) in the task's file: a Reason field,
//! written at schema 3 so an earlier build reads the file as a newer
//! quecto's rather than misread it; a task with none keeps the schema its
//! description's form picks, the bytes master wrote.
use super::super::{decode, encode};
use super::*;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

const SECTIONED_TEXT: &str = "## Goal\nShip it.\n\n## Done when\n- [ ] Merged";

const BACK_TO_READY: &str = r#"<!-- quecto-task schema=3 -->
# Ship it

| Field | Value |
|---|---|
| Id | ship |
| Kind | task |
| Status | ready |
| Reason | The pull request was closed: the approach changed |
| Created | 2026-10-11T09:30:00Z |
| Updated | 2026-10-11T09:30:00Z |

## Description
Refresh PR #2652.
"#;

const DONE_WITHOUT_RUN: &str = r#"<!-- quecto-task schema=3 -->
# Ship it

| Field | Value |
|---|---|
| Id | ship |
| Kind | task |
| Status | done |
| Resolution | done_without_run |
| Reason | Answered in chat \| no change |
| Created | 2026-10-11T09:30:00Z |
| Updated | 2026-10-11T09:30:00Z |

## Goal
Ship it.

## Done when
| Done | Outcome |
|---|---|
| no | Merged |
"#;

fn fields(status: TaskStatus, reason: &str) -> TaskFields {
    let at = Timestamp::parse("2026-10-11T09:30:00Z").unwrap();
    TaskFields {
        id: Slug::parse("ship").unwrap(),
        title: "Ship it".into(),
        kind: TaskKind::Task,
        description: "Refresh PR #2652.".into(),
        description_form: DescriptionForm::Legacy,
        status,
        parent: None,
        depends_on: vec![],
        accepted_plan: None,
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at.clone(),
        updated: at,
        reason: Some(reason.into()),
    }
}

#[test]
fn a_ready_tasks_reason_round_trips_at_schema_3() {
    let reason = "The pull request was closed: the approach changed";
    let task = Task::new(fields(TaskStatus::Ready, reason)).unwrap();
    let file = encode::<TaskCodec>(&task).unwrap();
    assert_eq!(file, BACK_TO_READY);
    let read = decode::<TaskCodec>(file.as_bytes()).unwrap();
    assert_eq!(read, task);
    assert_eq!(read.fields().description_form, DescriptionForm::Legacy);
}

#[test]
fn a_task_done_without_a_run_round_trips_sectioned_at_schema_3() {
    let task = Task::new(TaskFields {
        description: SECTIONED_TEXT.into(),
        description_form: DescriptionForm::Sectioned,
        resolution: Some(Resolution::DoneWithoutRun),
        ..fields(TaskStatus::Done, "Answered in chat | no change")
    })
    .unwrap();
    let file = encode::<TaskCodec>(&task).unwrap();
    assert_eq!(
        file,
        DONE_WITHOUT_RUN.replace("| no | Merged |", "| yes | Merged |")
    );
    let read = decode::<TaskCodec>(file.as_bytes()).unwrap();
    assert_eq!(read, task);
    assert_eq!(read.fields().description_form, DescriptionForm::Sectioned);
}

#[test]
fn a_task_without_a_reason_keeps_its_forms_schema() {
    for (form, text, schema) in [
        (DescriptionForm::Legacy, "Refresh PR #2652.", 1),
        (DescriptionForm::Sectioned, SECTIONED_TEXT, 2),
    ] {
        let task = Task::new(TaskFields {
            description: text.into(),
            description_form: form,
            reason: None,
            ..fields(TaskStatus::Ready, "unused")
        })
        .unwrap();
        let file = encode::<TaskCodec>(&task).unwrap();
        assert!(
            file.starts_with(&format!("<!-- quecto-task schema={schema} -->\n")),
            "{form:?}: {file}"
        );
        assert!(!file.contains("| Reason |"), "{form:?}");
    }
}
