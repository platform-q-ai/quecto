//! The review codec: the example file byte for byte, rounds and findings
//! read back, State as the last verdict, findings in round order and
//! numbered without a gap, a SHA-256 commit, and the widest review inside
//! the caps. Every file is read by the CommonMark/GFM oracle cell by cell.
use super::super::{CodecError, decode, encode};
use super::*;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::review::{
    MAX_FINDINGS, MAX_REVIEW_TEXT_BYTES, MAX_ROUNDS,
};
use crate::domain::project_board::entities::task_parts::{MAX_FINDING_CHARS, MAX_LINE_CHARS};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::infrastructure::persistence::project_board::markdown::tests::oracle::outline_in;
use crate::infrastructure::persistence::project_board::markdown::{Document, parse};

const EXAMPLE: &str = r#"<!-- quecto-review schema=1 -->
# Review of change 2 of board-cli

| Field | Value |
|---|---|
| Task | board-cli |
| Change | 2 |
| State | approved |
| Created | 2026-10-11T12:00:00Z |
| Updated | 2026-10-11T15:30:00Z |

## Rounds
| Round | Commit | Reviewer | At | Verdict |
|---|---|---|---|---|
| 1 | 0123456789abcdef0123456789abcdef01234567 | Rex (`rex@example.com`) | 2026-10-11T12:00:00Z | changes_requested |
| 2 | 89abcdef0123456789abcdef0123456789abcdef | Rex (`rex@example.com`) | 2026-10-11T15:30:00Z | approved |

## Findings
| Round | Finding | Severity | Location | Text |
|---|---|---|---|---|
| 1 | 1 | high | `src/store.rs:42` | A lost write is read as `Ok`; the `a\|b` case too |
| 1 | 2 | low | `-` | The doc names the old flag |
"#;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn at(text: &str) -> Timestamp {
    Timestamp::parse(text).unwrap()
}

fn rex() -> Identity {
    Identity {
        name: "Rex".into(),
        email: "rex@example.com".into(),
    }
}

fn finding(severity: Severity, location: Option<&str>, text: &str) -> Finding {
    Finding {
        location: location.map(str::to_string),
        severity,
        text: text.into(),
    }
}

fn round(commit: &str, when: &str, verdict: ReviewVerdict, findings: Vec<Finding>) -> Round {
    Round {
        commit: CommitId::parse(commit).unwrap(),
        reviewer: rex(),
        at: at(when),
        verdict,
        findings,
    }
}

fn example() -> ReviewFields {
    ReviewFields {
        task: slug("board-cli"),
        change: 2,
        rounds: vec![
            round(
                "0123456789abcdef0123456789abcdef01234567",
                "2026-10-11T12:00:00Z",
                ReviewVerdict::ChangesRequested,
                vec![
                    finding(
                        Severity::High,
                        Some("src/store.rs:42"),
                        "A lost write is read as `Ok`; the `a|b` case too",
                    ),
                    finding(Severity::Low, None, "The doc names the old flag"),
                ],
            ),
            round(
                "89abcdef0123456789abcdef0123456789abcdef",
                "2026-10-11T15:30:00Z",
                ReviewVerdict::Approved,
                vec![],
            ),
        ],
        created: at("2026-10-11T12:00:00Z"),
        updated: at("2026-10-11T15:30:00Z"),
    }
}

/// `fields` written and read back as itself, every cell as a reader shows
/// it; the file.
fn round_trips(fields: ReviewFields) -> String {
    let review = Review::new(fields).unwrap();
    let file = encode::<ReviewCodec>(&review).unwrap();
    assert_eq!(decode::<ReviewCodec>(file.as_bytes()), Ok(review), "{file}");
    let document: Document = parse(file.as_bytes(), &ReviewCodec::SHAPE).unwrap();
    assert_eq!(outline_in(&file, &document, &ReviewCodec::SHAPE), Ok(()));
    file
}

/// The example file with `from` replaced by `to` (once).
fn edited(from: &str, to: &str) -> String {
    assert!(EXAMPLE.contains(from), "{from:?}");
    EXAMPLE.replacen(from, to, 1)
}

fn refused_at(file: &str, at: &str) {
    let read = decode::<ReviewCodec>(file.as_bytes());
    assert!(
        matches!(&read, Err(CodecError::Cell { field, .. }) if *field == at),
        "{read:?}\n{file}"
    );
}

#[test]
fn the_example_review_file_is_byte_exact() {
    assert_eq!(round_trips(example()), EXAMPLE);
}

#[test]
fn rounds_and_findings_round_trip() {
    let mut fields = example();
    // Findings in every round, the last approving with a critical one.
    fields.rounds[1].findings = vec![
        finding(
            Severity::Critical,
            Some(".github/ci.yml:3:7"),
            "**Still** open",
        ),
        finding(
            Severity::Medium,
            Some("@scope/__init__.py:1"),
            "www-free text",
        ),
        finding(Severity::Low, None, "x"),
    ];
    let file = round_trips(fields.clone());
    assert!(file.contains("| 2 | 3 | low | `-` | x |"), "{file}");
    // One round, no findings: no Findings table.
    fields.rounds.truncate(1);
    fields.rounds[0].findings.clear();
    fields.updated = fields.created.clone();
    let bare = round_trips(fields);
    assert!(!bare.contains("## Findings"), "{bare}");
}

#[test]
fn a_state_that_is_not_the_last_verdict_is_not_canonical() {
    let wrong = edited("| State | approved |", "| State | changes_requested |");
    assert_eq!(
        decode::<ReviewCodec>(wrong.as_bytes()),
        Err(CodecError::NotCanonical)
    );
    refused_at(&edited("| State | approved |", "| State | done |"), "State");
}

#[test]
fn findings_out_of_round_order_are_refused() {
    let mut fields = example();
    fields.rounds[1].findings = vec![finding(Severity::Low, None, "later")];
    let file = round_trips(fields);
    let later = "| 2 | 1 | low | `-` | later |\n";
    let first = "| 1 | 1 | high |";
    let moved = file
        .replacen(later, "", 1)
        .replacen(first, &format!("{}{first}", later), 1);
    assert!(moved.contains("| 2 | 1 | low |"), "{moved}");
    refused_at(&moved, "Findings");
    // A round the review does not have.
    refused_at(&edited("| 1 | 2 | low |", "| 3 | 1 | low |"), "Findings");
}

#[test]
fn a_gap_in_finding_numbers_is_refused() {
    refused_at(&edited("| 1 | 2 | low |", "| 1 | 3 | low |"), "Findings");
    refused_at(&edited("| 1 | 1 | high |", "| 1 | 0 | high |"), "Findings");
    // Rounds are numbered from 1 in order too.
    refused_at(&edited("| 2 | 89ab", "| 3 | 89ab"), "Rounds");
}

#[test]
fn a_sha256_commit_round_trips() {
    let mut fields = example();
    fields.rounds[1].commit = CommitId::parse(&"ab".repeat(32)).unwrap();
    let file = round_trips(fields);
    assert!(file.contains(&"ab".repeat(32)), "{file}");
    // Not a full commit id.
    refused_at(
        &edited("89abcdef0123456789abcdef0123456789abcdef", "89abcdef"),
        "Rounds",
    );
}

/// The review the domain admits at its widest, its findings' text as
/// `text` gives it for (round, finding).
fn widest(
    text: impl Fn(usize, usize) -> String,
    location: impl Fn(usize, usize) -> Option<String>,
) -> ReviewFields {
    let pipes = |count: usize| "|".repeat(count);
    // The widest name: 4-byte letters, each written as it is.
    let reviewer = Identity {
        name: "\u{1d538}".repeat(100),
        email: format!("{}@{}", pipes(126), pipes(127)),
    };
    let rounds = (0..MAX_ROUNDS)
        .map(|index| Round {
            commit: CommitId::parse(&"f".repeat(64)).unwrap(),
            reviewer: reviewer.clone(),
            at: at("9999-12-31T23:59:59Z"),
            verdict: ReviewVerdict::ChangesRequested,
            findings: (0..MAX_FINDINGS)
                .map(|finding| Finding {
                    location: location(index, finding),
                    severity: Severity::Critical,
                    text: text(index, finding),
                })
                .collect(),
        })
        .collect();
    ReviewFields {
        task: slug(&format!("{}x", "a".repeat(63))),
        change: 999,
        rounds,
        created: at("9999-12-31T23:59:59Z"),
        updated: at("9999-12-31T23:59:59Z"),
    }
}

#[test]
fn the_widest_review_fits() {
    // Most bytes and lines: every finding, the whole text budget spent on
    // `|`, written `\|`; the reviewer's name 4-byte letters and email all
    // `|`, written `\|`.
    let findings = MAX_ROUNDS * MAX_FINDINGS;
    let share = MAX_REVIEW_TEXT_BYTES / findings;
    let most = widest(|_, _| "|".repeat(share), |_, _| None);
    assert_eq!(most.text_bytes(), MAX_REVIEW_TEXT_BYTES);
    // The longest line: one finding of 4-byte letters at its longest, at
    // the longest place, numbered widest (round 64, finding 128).
    let place = format!("{}:999999999:999999999", "a".repeat(MAX_LINE_CHARS - 20));
    let last = (MAX_ROUNDS - 1, MAX_FINDINGS - 1);
    let longest_line = widest(
        |round, finding| match (round, finding) == last {
            true => "\u{1d538}".repeat(MAX_FINDING_CHARS),
            false => "e".into(),
        },
        |round, finding| ((round, finding) == last).then(|| place.clone()),
    );
    let mut seen = (0, 0, 0);
    for fields in [most, longest_line] {
        let file = round_trips(fields);
        let longest = file.lines().map(str::len).max().unwrap();
        seen = (
            seen.0.max(file.len()),
            seen.1.max(file.lines().count()),
            seen.2.max(longest),
        );
    }
    let (bytes, lines, longest) = seen;
    // Pinned, so a change to the codec's rows is seen: 64 rounds of 128
    // findings, 8,274 lines; the budget as `\|`, 984,178 bytes; the
    // longest finding at the longest place in round 64's finding 128, a
    // 16,531-byte line.
    assert_eq!((bytes, lines, longest), (984_178, 8_274, 16_531));
    assert!(bytes <= REVIEW_LIMITS.file_bytes, "{bytes}");
    assert!(lines <= REVIEW_LIMITS.lines, "{lines}");
    assert!(longest <= REVIEW_LIMITS.line_bytes, "{longest}");
}
