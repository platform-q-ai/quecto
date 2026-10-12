//! The task codec: the example file byte for byte, every shape a task
//! takes, the claim's three rows, hand edits, and the caps.

const EXAMPLE: &str = r#"<!-- quecto-task schema=1 -->
# The board CLI answers in `JSON`

| Field | Value |
|---|---|
| Id | board-cli |
| Kind | task |
| Status | in_progress |
| Parent | boards |
| Accepted plan | 2 |
| Claimed by | Shane (work) (`shane@example.com`) |
| Claimed since | 2026-10-11T12:00:00Z |
| Claim expires | 2026-10-11T14:00:00Z |
| GitHub issue | #2481 |
| Created | 2026-10-11T09:30:00Z |
| Updated | 2026-10-11T12:00:00Z |

## Description
Build the CLI on the store.
\# not a heading

- read first
- then write

```sh
quecto board list

quecto board show board-cli
```

## Depends on
| Task |
|---|
| store |
| schema |

## Superseded by
| Task |
|---|
| board-cli-v2 |

## Pull requests
| Pull request |
|---|
| #2490 |
| #2491 |

## References
| Reference |
|---|
| `#2470` |
| `platform-q-ai/browser_task#18` |
| `https://example.com/a_b%20c#top` |
"#;
use super::super::{CodecError, decode, encode};
use super::*;
use crate::domain::project_board::entities::claim::{Identity, MAX_EMAIL_BYTES, MAX_NAME_CHARS};
use crate::domain::project_board::entities::plan::MAX_PLAN_VERSION;
use crate::domain::project_board::entities::task::{
    MAX_DEPENDENCIES, MAX_DESCRIPTION_CHARS, MAX_PRS, MAX_REFERENCES, MAX_SUPERSEDED_BY,
    MAX_TEXT_BYTES, MAX_TITLE_CHARS,
};
use crate::domain::project_board::value_objects::reference::{
    MAX_ISSUE_NUMBER, MAX_REFERENCE_BYTES,
};
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::infrastructure::persistence::project_board::markdown::MarkdownError;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn at(text: &str) -> Timestamp {
    Timestamp::parse(text).unwrap()
}

fn person(name: &str, email: &str) -> Identity {
    Identity {
        name: name.into(),
        email: email.into(),
    }
}

fn example() -> TaskFields {
    TaskFields {
        id: slug("board-cli"),
        title: "The board CLI answers in `JSON`".into(),
        kind: TaskKind::Task,
        description:
            "Build the CLI on the store.\n# not a heading\n\n- read first\n- then write\n\n\
                      ```sh\nquecto board list\n\nquecto board show board-cli\n```"
                .into(),
        status: TaskStatus::InProgress,
        parent: Some(slug("boards")),
        depends_on: vec![slug("store"), slug("schema")],
        accepted_plan: Some(2),
        claim: Some(Claim {
            holder: person("Shane (work)", "shane@example.com"),
            since: at("2026-10-11T12:00:00Z"),
            expires: at("2026-10-11T14:00:00Z"),
        }),
        prs: vec![2490, 2491],
        github_issue: Some(IssueNumber::new(2481).unwrap()),
        resolution: None,
        superseded_by: vec![slug("board-cli-v2")],
        references: [
            "#2470",
            "platform-q-ai/browser_task#18",
            "https://example.com/a_b%20c#top",
        ]
        .iter()
        .map(|text| Reference::parse(text).unwrap())
        .collect(),
        created: at("2026-10-11T09:30:00Z"),
        updated: at("2026-10-11T12:00:00Z"),
    }
}

fn task(fields: TaskFields) -> Task {
    Task::new(fields).unwrap()
}

/// The example file with `from` replaced by `to` (once).
fn edited(from: &str, to: &str) -> String {
    assert!(EXAMPLE.contains(from), "{from:?}");
    EXAMPLE.replacen(from, to, 1)
}

#[test]
fn the_example_task_file_is_byte_exact() {
    let example = task(example());
    assert_eq!(encode::<TaskCodec>(&example).unwrap(), EXAMPLE);
    assert_eq!(decode::<TaskCodec>(EXAMPLE.as_bytes()), Ok(example));
}

/// Every status (held with its claim, finished with its resolution), every
/// kind, and the barest task of each, round trip.
#[test]
fn every_task_shape_round_trips() {
    let claim = example().claim;
    let bare = |status: TaskStatus| TaskFields {
        status,
        description: String::new(),
        parent: None,
        depends_on: vec![],
        accepted_plan: None,
        claim: None,
        prs: vec![],
        github_issue: None,
        superseded_by: vec![],
        references: vec![],
        ..example()
    };
    let mut shapes = vec![];
    for status in TaskStatus::ALL.iter().copied() {
        // Every status that may hold a claim holds one here.
        let held = match status.may_hold_claim() {
            true => claim.clone(),
            false => None,
        };
        let resolution = match status {
            TaskStatus::Done => Some(Resolution::Completed),
            _ => None,
        };
        shapes.push(TaskFields {
            claim: held.clone(),
            resolution,
            ..bare(status)
        });
        let finished = matches!(status, TaskStatus::Done | TaskStatus::Archived);
        shapes.push(TaskFields {
            status,
            claim: held,
            resolution,
            superseded_by: match finished {
                true => vec![],
                false => example().superseded_by,
            },
            ..example()
        });
    }
    for resolution in Resolution::ALL.iter().copied() {
        // A superseded or duplicate task names what took it over.
        let named = matches!(resolution, Resolution::Superseded | Resolution::Duplicate);
        shapes.push(TaskFields {
            resolution: Some(resolution),
            references: match named {
                true => example().references,
                false => vec![],
            },
            ..bare(TaskStatus::Archived)
        });
    }
    shapes.push(TaskFields {
        resolution: Some(Resolution::Superseded),
        claim: None,
        status: TaskStatus::Archived,
        ..example()
    });
    for kind in TaskKind::ALL.iter().copied() {
        shapes.push(TaskFields {
            kind,
            ..bare(TaskStatus::Draft)
        });
    }
    for fields in shapes {
        let task = task(fields);
        let file = encode::<TaskCodec>(&task).unwrap();
        assert_eq!(decode::<TaskCodec>(file.as_bytes()), Ok(task), "{file}");
    }
}

#[test]
fn a_hand_edited_task_file_is_refused() {
    use MarkdownError as E;
    let structure =
        |line: usize, expected: &'static str| CodecError::Markdown(E::Structure { line, expected });
    let cases = [
        // Rows out of the shape's order.
        (
            edited(
                "| Kind | task |\n| Status | in_progress |\n",
                "| Status | in_progress |\n| Kind | task |\n",
            ),
            structure(8, "fields in the kind's order"),
        ),
        // No blank line before a section.
        (
            edited("```\n\n## Depends on", "```\n## Depends on"),
            structure(30, "a blank line between blocks"),
        ),
        // A status as a reader might spell it: no ASCII cell holds a space.
        (
            edited("| Status | in_progress |", "| Status | In progress |"),
            CodecError::Markdown(E::Cell {
                line: 8,
                why: "an id or pointer of ASCII letters, digits and ._:/#@+_-",
            }),
        ),
        // A table with no rows: a missing section is "none", so a file
        // writing one is not the task's own.
        (
            edited(
                "## Depends on\n| Task |\n|---|\n| store |\n| schema |\n",
                "## Depends on\n| Task |\n|---|\n",
            ),
            CodecError::NotCanonical,
        ),
        // Sections out of order: Superseded by after References.
        (
            format!(
                "{}\n## Superseded by\n| Task |\n|---|\n| board-cli-v2 |\n",
                edited(
                    "## Superseded by\n| Task |\n|---|\n| board-cli-v2 |\n\n",
                    ""
                )
            ),
            structure(49, "sections in the kind's order"),
        ),
        // A section the kind has not got: the task's old review findings.
        (
            format!("{EXAMPLE}\n## Reviews\n| Task |\n|---|\n| r1 |\n"),
            CodecError::Markdown(E::UnknownSection {
                heading: "Reviews".into(),
            }),
        ),
    ];
    for (index, (file, refusal)) in cases.into_iter().enumerate() {
        assert_eq!(
            decode::<TaskCodec>(file.as_bytes()),
            Err(refusal),
            "case {index}"
        );
    }
}

#[test]
fn a_claim_needs_all_three_rows() {
    for row in [
        "| Claimed by | Shane (work) (`shane@example.com`) |\n",
        "| Claimed since | 2026-10-11T12:00:00Z |\n",
        "| Claim expires | 2026-10-11T14:00:00Z |\n",
    ] {
        let read = decode::<TaskCodec>(edited(row, "").as_bytes());
        assert!(
            matches!(
                &read,
                Err(CodecError::Cell {
                    field: "Claimed by",
                    ..
                })
            ),
            "{row}: {read:?}"
        );
    }
}

#[test]
fn a_newer_task_schema_asks_for_an_upgrade() {
    let newer = edited("quecto-task schema=1", "quecto-task schema=2");
    assert_eq!(
        decode::<TaskCodec>(newer.as_bytes()),
        Err(CodecError::NewerSchema { found: 2, known: 1 })
    );
    let other = edited("quecto-task schema=1", "quecto-plan schema=1");
    assert_eq!(
        decode::<TaskCodec>(other.as_bytes()),
        Err(CodecError::Schema)
    );
}

/// A slug of 64 bytes, numbered.
fn long_slug(index: usize) -> Slug {
    slug(&format!("{}{index:03}", "a".repeat(61)))
}

/// References summing to `bytes` exactly: distinct https URLs of at most
/// [`MAX_REFERENCE_BYTES`].
fn references_of(mut bytes: usize) -> Vec<Reference> {
    let mut references = vec![];
    while bytes > 0 {
        let head = format!("https://example.com/{}/", references.len());
        let len = bytes.min(MAX_REFERENCE_BYTES);
        let text = format!("{head}{}", "a".repeat(len - head.len()));
        references.push(Reference::parse(&text).unwrap());
        bytes -= len;
    }
    references
}

/// The widest structure the domain admits around `title`, `description`
/// and the references that fill what is left of the text budget: every
/// list full, every optional row set, every id its longest, and a holder
/// of four-byte letters and an email of `|`s, which a cell writes `\|`.
fn widest(title: String, description: String) -> TaskFields {
    let name = "\u{1d538}".repeat(MAX_NAME_CHARS);
    let email = format!("{}@b", "|".repeat(MAX_EMAIL_BYTES - 2));
    let mut fields = TaskFields {
        id: long_slug(999),
        title,
        kind: TaskKind::Chore,
        description,
        status: TaskStatus::InProgress,
        parent: Some(long_slug(998)),
        depends_on: (0..MAX_DEPENDENCIES).map(long_slug).collect(),
        accepted_plan: Some(MAX_PLAN_VERSION),
        claim: Some(Claim {
            holder: person(&name, &email),
            since: at("2026-10-11T12:00:00Z"),
            expires: at("2026-10-11T14:00:00Z"),
        }),
        prs: (0..MAX_PRS as u64).map(|n| u64::MAX - n).collect(),
        github_issue: Some(IssueNumber::new(MAX_ISSUE_NUMBER).unwrap()),
        resolution: None,
        superseded_by: (100..100 + MAX_SUPERSEDED_BY).map(long_slug).collect(),
        references: vec![],
        created: at("2026-10-11T09:30:00Z"),
        updated: at("2026-10-11T12:00:00Z"),
    };
    let left = MAX_TEXT_BYTES - fields.text_bytes();
    let left = left.min(MAX_REFERENCES * MAX_REFERENCE_BYTES);
    fields.references = references_of(left);
    fields
}

#[test]
fn the_widest_task_fits_its_limits() {
    let letters = |count: usize| "\u{1d538}".repeat(count);
    // Most lines: the description all blank lines inside a fence.
    let fenced = format!("```{}```", "\n".repeat(MAX_DESCRIPTION_CHARS - 6));
    let most_lines = widest("t".into(), fenced);
    // The longest line: the title and description at their widest in
    // four-byte letters, the references taking the rest of the budget.
    let longest_line = widest(letters(MAX_TITLE_CHARS), letters(MAX_DESCRIPTION_CHARS));
    // Most bytes: as many description lines as can start with `#` (each
    // written `\#`, a byte more) while the budget is still spent whole:
    // `#` lines take 2 characters and 2 bytes each, letters 1 and 4, so
    // each `#` line gives up 6 bytes of letters and the references (at
    // most 64 × 2,048) must make them up.
    let escaped_lines = |escaped: usize| {
        let description = format!(
            "{}{}",
            "#\n".repeat(escaped),
            letters(MAX_DESCRIPTION_CHARS - 2 * escaped)
        );
        widest(letters(MAX_TITLE_CHARS), description)
    };
    let most_bytes = escaped_lines(11_093);
    assert!(
        escaped_lines(11_094).text_bytes() < MAX_TEXT_BYTES,
        "one more `#` line and the references cannot make the budget up"
    );
    for fields in [&longest_line, &most_bytes] {
        assert_eq!(fields.text_bytes(), MAX_TEXT_BYTES, "the whole budget");
    }
    let measured = [most_lines, longest_line, most_bytes].map(|fields| {
        let task = task(fields);
        let file = encode::<TaskCodec>(&task).unwrap();
        assert_eq!(decode::<TaskCodec>(file.as_bytes()), Ok(task));
        let longest = file.lines().map(str::len).max().unwrap();
        (file.len(), file.lines().count(), longest)
    });
    let [most_lines, longest_line, most_bytes] = measured;
    assert_eq!(most_lines.1, 65_725, "the most lines");
    assert_eq!(longest_line.2, 262_144, "the longest line");
    assert_eq!(most_bytes.0, 346_705, "the most bytes");
    let limits = TASK_LIMITS;
    for (bytes, lines, longest) in measured {
        assert!(
            bytes <= limits.file_bytes && lines <= limits.lines && longest <= limits.line_bytes
        );
    }
}

/// Whether an error names the cap a file is past.
type Over = fn(&CodecError) -> bool;

#[test]
fn one_byte_or_line_over_a_limit_is_refused() {
    let marker = "<!-- quecto-task schema=1 -->\n";
    // Lines of at most a thousand bytes, so only the size is past a cap.
    let sized = |bytes: usize| {
        let mut file = marker.to_string();
        while file.len() < bytes {
            let line = (bytes - file.len()).min(1_000);
            file.push_str(&format!("{}\n", "x".repeat(line - 1)));
        }
        file
    };
    let lined = |lines: usize| format!("{marker}{}", "x\n".repeat(lines - 1));
    let long_line = |bytes: usize| format!("{marker}# {}\n", "x".repeat(bytes - 2));
    use MarkdownError as E;
    let limits = TASK_LIMITS;
    let cases: [(String, Over); 3] = [
        (sized(limits.file_bytes + 1), |e| {
            matches!(e, CodecError::Markdown(E::TooLarge { .. }))
        }),
        (lined(limits.lines + 1), |e| {
            matches!(e, CodecError::Markdown(E::TooManyLines { .. }))
        }),
        (long_line(limits.line_bytes + 1), |e| {
            matches!(e, CodecError::Markdown(E::LineTooLong { .. }))
        }),
    ];
    for (file, over) in cases {
        let refused = decode::<TaskCodec>(file.as_bytes()).unwrap_err();
        assert!(over(&refused), "{refused:?}");
    }
    // At each limit the file is refused for what it holds, not its size.
    for file in [
        sized(limits.file_bytes),
        lined(limits.lines),
        long_line(limits.line_bytes),
    ] {
        let refused = decode::<TaskCodec>(file.as_bytes()).unwrap_err();
        let sized = matches!(
            refused,
            CodecError::Markdown(
                E::TooLarge { .. } | E::TooManyLines { .. } | E::LineTooLong { .. }
            )
        );
        assert!(!sized, "{refused:?}");
    }
}

/// An archived task others took over: no claim, its resolution row.
fn archived() -> TaskFields {
    TaskFields {
        status: TaskStatus::Archived,
        claim: None,
        resolution: Some(Resolution::Superseded),
        description: "Moved to the v2 CLI.".into(),
        depends_on: vec![],
        prs: vec![],
        references: vec![Reference::parse("#2470").unwrap()],
        updated: at("2026-10-12T08:00:00Z"),
        ..example()
    }
}

const ARCHIVED: &str = r#"<!-- quecto-task schema=1 -->
# The board CLI answers in `JSON`

| Field | Value |
|---|---|
| Id | board-cli |
| Kind | task |
| Status | archived |
| Parent | boards |
| Accepted plan | 2 |
| GitHub issue | #2481 |
| Resolution | superseded |
| Created | 2026-10-11T09:30:00Z |
| Updated | 2026-10-12T08:00:00Z |

## Description
Moved to the v2 CLI.

## Superseded by
| Task |
|---|
| board-cli-v2 |

## References
| Reference |
|---|
| `#2470` |
"#;

#[test]
fn the_archived_task_file_is_byte_exact() {
    let archived = task(archived());
    assert_eq!(encode::<TaskCodec>(&archived).unwrap(), ARCHIVED);
    assert_eq!(decode::<TaskCodec>(ARCHIVED.as_bytes()), Ok(archived));
}

/// A reference keeps the form it was written in, whatever its key: the
/// repository's case, a URL's host, port, dot segments, escapes and empty
/// query or fragment.
#[test]
fn a_reference_keeps_its_written_form() {
    let written = [
        "Platform-Q-AI/Browser_Task#18",
        "https://EXAMPLE.com:443/a/./b/%7e?#",
    ];
    let fields = TaskFields {
        references: written
            .iter()
            .map(|text| Reference::parse(text).unwrap())
            .collect(),
        ..example()
    };
    let file = encode::<TaskCodec>(&task(fields)).unwrap();
    for text in written {
        assert!(file.contains(&format!("| `{text}` |\n")), "{file}");
    }
    let read = decode::<TaskCodec>(file.as_bytes()).unwrap();
    let shown: Vec<&str> = read
        .fields()
        .references
        .iter()
        .map(Reference::as_str)
        .collect();
    assert_eq!(shown, written);
}

/// A title its heading cannot hold is refused when written, naming the
/// title: closing hashes, a space at an edge, raw HTML, a character
/// reference, dollar math.
#[test]
fn a_title_its_heading_cannot_hold_is_refused() {
    for title in ["a #", " a", "a\u{a0}", "a <b>", "a &amp;", "$x$"] {
        let titled = TaskFields {
            title: title.into(),
            ..example()
        };
        let refused = encode::<TaskCodec>(&task(titled));
        assert!(
            matches!(&refused, Err(CodecError::Cell { field: "Title", .. })),
            "{title:?}: {refused:?}"
        );
    }
}
