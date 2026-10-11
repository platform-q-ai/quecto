//! Where a task came from and how it ended (#2481 S8). How a reference
//! itself is judged and keyed is the value object's (`reference_tests`).
use super::tests::{fields, slug};
use super::*;
use crate::domain::project_board::value_objects::vocabulary::Resolution::*;

/// One change that must make valid fields invalid.
type Edit = fn(&mut TaskFields);

fn issue(number: u64) -> IssueNumber {
    IssueNumber::new(number).unwrap()
}

fn refs(texts: &[&str]) -> Vec<Reference> {
    texts
        .iter()
        .map(|text| Reference::parse(text).unwrap())
        .collect()
}

/// An archived task superseded by another, from an issue, pointing at
/// work outside the board.
fn closed() -> TaskFields {
    TaskFields {
        status: TaskStatus::Archived,
        claim: None,
        github_issue: Some(issue(2481)),
        resolution: Some(Superseded),
        superseded_by: vec![slug("board-store-v2")],
        references: refs(&[
            "#2470",
            "platform-q-ai/.github#7",
            "https://example.com:8443/a/b?c=d&e=f#g",
        ]),
        ..fields()
    }
}

/// `fields()` at `status`, holding no claim where it may not.
fn at_status(status: TaskStatus) -> TaskFields {
    let claim = status.may_hold_claim().then(|| fields().claim).flatten();
    TaskFields {
        status,
        claim,
        ..fields()
    }
}

fn refused(task: TaskFields) -> String {
    Task::new(task).unwrap_err().field
}

#[test]
fn an_archived_task_holds_where_it_came_from_and_how_it_ended() {
    assert!(Task::new(closed()).is_ok());
    let open = TaskFields {
        resolution: None,
        github_issue: Some(issue(1)),
        superseded_by: vec![slug("next")],
        references: refs(&["#2"]),
        ..fields()
    };
    assert!(
        Task::new(open).is_ok(),
        "only the resolution waits for the end"
    );
}

/// Every resolution at every status, with and without what took the task
/// over, against the rule written out independently of the code: a done
/// task is completed and superseded by none, an archived one anything, any
/// other unresolved; `superseded` and `duplicate` name a task or a
/// reference; a task superseded by others is unresolved, superseded or
/// duplicate.
#[test]
fn a_resolution_fits_its_status_and_what_took_the_task_over() {
    let resolutions = [
        None,
        Some(Completed),
        Some(NotPlanned),
        Some(Duplicate),
        Some(Superseded),
    ];
    for status in [
        TaskStatus::Ready,
        TaskStatus::Review,
        TaskStatus::Done,
        TaskStatus::Archived,
    ] {
        for resolution in resolutions {
            for (by, with_refs) in [(false, false), (true, false), (false, true), (true, true)] {
                let task = TaskFields {
                    resolution,
                    superseded_by: if by { vec![slug("next")] } else { vec![] },
                    references: if with_refs { refs(&["#9"]) } else { vec![] },
                    ..at_status(status)
                };
                let by_status = matches!(
                    (resolution, status),
                    (Some(Completed), TaskStatus::Done)
                        | (Some(_), TaskStatus::Archived)
                        | (
                            None,
                            TaskStatus::Ready | TaskStatus::Review | TaskStatus::Archived
                        )
                );
                let names =
                    matches!(resolution, Some(Duplicate | Superseded)).then_some(by || with_refs);
                let fits = !by || matches!(resolution, None | Some(Duplicate | Superseded));
                let valid = by_status && names.unwrap_or(true) && fits;
                let built = Task::new(task);
                assert_eq!(
                    built
                        .as_ref()
                        .map(|_| ())
                        .map_err(|error| error.field.as_str()),
                    if valid { Ok(()) } else { Err("resolution") },
                    "{status:?} {resolution:?} superseded_by={by} references={with_refs}"
                );
            }
        }
    }
}

/// An issue is never one of the task's own pull requests (reported where
/// the pull request is listed), and neither is named by `#n` in its
/// references.
#[test]
fn a_tasks_issue_and_pull_requests_are_its_own() {
    let clash = TaskFields {
        prs: vec![1, 2481],
        ..closed()
    };
    assert_eq!(refused(clash), "prs/1");
    let own_issue = TaskFields {
        references: refs(&["#1", "#2481"]),
        ..closed()
    };
    assert_eq!(refused(own_issue), "references/1");
    let own_pr = TaskFields {
        prs: vec![2482],
        references: refs(&["#2482"]),
        ..closed()
    };
    assert_eq!(refused(own_pr), "references/0");
    let elsewhere = TaskFields {
        prs: vec![2482],
        references: refs(&["platform-q-ai/quecto#2481", "platform-q-ai/quecto#2482"]),
        ..closed()
    };
    assert!(
        Task::new(elsewhere).is_ok(),
        "the task does not know its repository"
    );
}

#[test]
fn the_tasks_that_superseded_it_are_others_named_once() {
    let cases: Vec<(&str, Edit)> = vec![
        ("superseded_by/0", |t| {
            t.superseded_by = vec![slug("board-store")]
        }),
        ("superseded_by", |t| {
            t.superseded_by = vec![slug("a"), slug("a")]
        }),
        ("superseded_by", |t| {
            t.superseded_by = (0..=MAX_SUPERSEDED_BY)
                .map(|n| slug(&format!("s{n}")))
                .collect();
        }),
        ("references", |t| {
            t.references = (1..=MAX_REFERENCES + 1)
                .map(|n| Reference::parse(&format!("o/r#{n}")).unwrap())
                .collect();
        }),
    ];
    for (field, edit) in cases {
        let mut broken = closed();
        edit(&mut broken);
        assert_eq!(refused(broken), field);
    }
}

/// A reference is named once by its key.
#[test]
fn a_reference_is_named_once_by_its_key() {
    for pair in [
        ["#1", "#1"],
        ["Owner/Repo#1", "owner/repo#1"],
        ["https://EXAMPLE.com", "https://example.com/"],
    ] {
        let task = TaskFields {
            references: refs(&pair),
            ..closed()
        };
        assert_eq!(refused(task), "references", "{pair:?}");
    }
    let two = TaskFields {
        references: refs(&["https://example.com/A", "https://example.com/a"]),
        ..closed()
    };
    assert!(Task::new(two).is_ok());
}

/// References are text: they count toward the task's budget.
#[test]
fn a_tasks_references_count_toward_its_text() {
    let without = TaskFields {
        references: vec![],
        ..closed()
    };
    let bytes: usize = closed()
        .references
        .iter()
        .map(|reference| reference.as_str().len())
        .sum();
    assert_eq!(closed().text_bytes(), without.text_bytes() + bytes);
}
