//! Where a task came from and how it ended (#2481 S8).
use super::tests::{fields, slug};
use super::*;
use crate::domain::project_board::entities::task_parts::{MAX_ISSUE_NUMBER, MAX_REFERENCE_BYTES};
use crate::domain::project_board::value_objects::vocabulary::Resolution::*;

/// An archived task superseded by another, from an issue, pointing at
/// work outside the board.
fn closed() -> TaskFields {
    TaskFields {
        status: TaskStatus::Archived,
        claim: None,
        github_issue: Some(2481),
        resolution: Some(Superseded),
        superseded_by: vec![slug("board-store-v2")],
        references: vec![
            "#2470".into(),
            "platform-q-ai/.github#7".into(),
            "https://example.com:8443/a/b?c=d&e=f#g".into(),
        ],
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

/// One change that must make valid fields invalid.
type Edit = fn(&mut TaskFields);

fn refused(task: TaskFields) -> String {
    Task::new(task).unwrap_err().field
}

#[test]
fn an_archived_task_holds_where_it_came_from_and_how_it_ended() {
    assert!(Task::new(closed()).is_ok());
    let open = TaskFields {
        resolution: None,
        github_issue: Some(1),
        superseded_by: vec![slug("next")],
        references: vec!["#2".into()],
        ..fields()
    };
    assert!(
        Task::new(open).is_ok(),
        "only the resolution waits for the end"
    );
}

/// Every resolution at every status, with and without what took the task
/// over, against the rule written out independently of the code: a done
/// task is completed or unresolved, an archived one anything, any other
/// unresolved; `superseded` and `duplicate` name a task or a reference;
/// a task superseded by others is unresolved, superseded or duplicate.
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
            for (by, refs) in [(false, false), (true, false), (false, true), (true, true)] {
                let task = TaskFields {
                    resolution,
                    superseded_by: if by { vec![slug("next")] } else { vec![] },
                    references: if refs { vec!["#9".into()] } else { vec![] },
                    ..at_status(status)
                };
                let by_status = matches!(
                    (resolution, status),
                    (None, _)
                        | (Some(Completed), TaskStatus::Done)
                        | (Some(_), TaskStatus::Archived)
                );
                let names =
                    matches!(resolution, Some(Duplicate | Superseded)).then_some(by || refs);
                let fits = !by || matches!(resolution, None | Some(Duplicate | Superseded));
                let valid = by_status && names.unwrap_or(true) && fits;
                let built = Task::new(task);
                assert_eq!(
                    built
                        .as_ref()
                        .map(|_| ())
                        .map_err(|error| error.field.as_str()),
                    if valid { Ok(()) } else { Err("resolution") },
                    "{status:?} {resolution:?} superseded_by={by} references={refs}"
                );
            }
        }
    }
}

/// An issue is numbered as a reference is, to ten digits, and is never
/// one of the task's own pull requests nor one of its references.
#[test]
fn a_tasks_issue_is_its_own() {
    let widest = TaskFields {
        github_issue: Some(MAX_ISSUE_NUMBER),
        ..closed()
    };
    assert!(Task::new(widest).is_ok());
    for issue in [0, MAX_ISSUE_NUMBER + 1, 2482] {
        let task = TaskFields {
            github_issue: Some(issue),
            ..closed()
        };
        assert_eq!(refused(task), "github_issue", "{issue}");
    }
    let own = TaskFields {
        references: vec!["#1".into(), "#2481".into()],
        ..closed()
    };
    assert_eq!(refused(own), "references/1");
    let elsewhere = TaskFields {
        references: vec!["platform-q-ai/quecto#2481".into()],
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
            t.references = (1..=MAX_REFERENCES + 1).map(|n| format!("#{n}")).collect();
        }),
    ];
    for (field, edit) in cases {
        let mut broken = closed();
        edit(&mut broken);
        assert_eq!(refused(broken), field);
    }
}

/// A reference is named once by its key: a repository in any case is one
/// repository, and so is a URL's host; a URL's path keeps its case.
#[test]
fn a_reference_is_named_once_by_its_key() {
    for pair in [
        ["#1", "#1"],
        ["Owner/Repo#1", "owner/repo#1"],
        ["https://EXAMPLE.com/a", "https://example.com/a"],
    ] {
        let task = TaskFields {
            references: pair.map(String::from).to_vec(),
            ..closed()
        };
        assert_eq!(refused(task), "references", "{pair:?}");
    }
    for pair in [
        ["https://example.com/A", "https://example.com/a"],
        ["#1", "owner/repo#1"],
        ["owner/repo#1", "owner/repo#2"],
    ] {
        let task = TaskFields {
            references: pair.map(String::from).to_vec(),
            ..closed()
        };
        assert!(Task::new(task).is_ok(), "{pair:?}");
    }
}

/// A reference is `#n`, `owner/repo#n` or an https URL: a host of
/// dot-joined labels, a port from 1 to 65535, then printable ASCII with
/// no space, `|`, backtick or `\`, whose `%`s escape.
#[test]
fn a_reference_is_an_issue_or_an_https_url() {
    let label = "a".repeat(63);
    let fine = [
        "#1".to_string(),
        "#9999999999".into(),
        "o/r#1".into(),
        "https://a".into(),
        "https://a-b.c1".into(),
        format!("https://{label}.com"),
        "https://a:1".into(),
        "https://a:65535/x".into(),
        "https://a/%2Fb%e9".into(),
        "https://a?q=1&r=%20".into(),
        "https://a#frag".into(),
        "https://a#123".into(),
        "https://a/b#42".into(),
    ];
    for text in fine {
        let task = TaskFields {
            references: vec![text.clone()],
            ..closed()
        };
        assert!(Task::new(task).is_ok(), "{text}");
    }
    let bad = [
        "#0".to_string(),
        "#01".into(),
        "#12345678901".into(),
        "2470".into(),
        "a/b/c#1".into(),
        "./repo#1".into(),
        "owner/..#1".into(),
        "own er/repo#1".into(),
        "http://example.com".into(),
        "https://".into(),
        "https://a..b".into(),
        "https://a.".into(),
        "https://a-".into(),
        "https://-a".into(),
        format!("https://{label}a.com"),
        "https://a:0".into(),
        "https://a:99999".into(),
        "https://a:".into(),
        "https://a:8x".into(),
        "https://a/%zz".into(),
        "https://a/%2".into(),
        "https://a/%".into(),
        "https://user@a".into(),
        "https://a/b c".into(),
        "https://a/b|c".into(),
        "https://a/b`c".into(),
        "https://a/b\\c".into(),
        "https://ex\u{e4}mple.com".into(),
        String::new(),
    ];
    for text in bad {
        let task = TaskFields {
            references: vec![text.clone()],
            ..closed()
        };
        assert_eq!(refused(task), "references/0", "{text:?}");
    }
    let url = format!(
        "https://example.com/{}",
        "a".repeat(MAX_REFERENCE_BYTES - 20)
    );
    assert_eq!(url.len(), MAX_REFERENCE_BYTES);
    let longest = TaskFields {
        references: vec![url.clone()],
        ..closed()
    };
    assert!(Task::new(longest).is_ok(), "the longest reference");
    let past = TaskFields {
        references: vec![format!("{url}a")],
        ..closed()
    };
    assert_eq!(refused(past), "references/0");
}

/// References are text: they count toward the task's budget.
#[test]
fn a_tasks_references_count_toward_its_text() {
    let without = TaskFields {
        references: vec![],
        ..closed()
    };
    let refs: usize = closed().references.iter().map(String::len).sum();
    assert_eq!(closed().text_bytes(), without.text_bytes() + refs);
}
