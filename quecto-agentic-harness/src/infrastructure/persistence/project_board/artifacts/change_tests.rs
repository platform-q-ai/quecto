//! The change codec: the example file byte for byte, every way a change
//! stands, the unlink pair, what a code cell cannot hold, and the widest
//! change inside the caps.
use super::super::{CodecError, decode, encode};
use super::*;
use crate::domain::project_board::entities::claim::{Identity, MAX_EMAIL_BYTES, MAX_NAME_CHARS};
use crate::domain::project_board::value_objects::code_ref::{
    MAX_BRANCH_BYTES, MAX_REPO_PART_BYTES,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::infrastructure::persistence::project_board::markdown::MarkdownError;

const EXAMPLE: &str = "<!-- quecto-change schema=1 -->
# Change 2 of board-cli

| Field | Value |
|---|---|
| Task | board-cli |
| Number | 2 |
| Repository | `platform-q-ai/quecto` |
| Branch | `feat/2481-board_cli` |
| Pull request | #2510 |
| Merge commit | 0123456789abcdef0123456789abcdef01234567 |
| Linked by | Shane (work) (`shane@example.com`) |
| Created | 2026-10-11T12:00:00Z |
| Updated | 2026-10-11T15:00:00Z |
";

fn at(text: &str) -> Timestamp {
    Timestamp::parse(text).unwrap()
}

fn person(name: &str, email: &str) -> Identity {
    Identity {
        name: name.into(),
        email: email.into(),
    }
}

fn example() -> ChangeFields {
    ChangeFields {
        task: Slug::parse("board-cli").unwrap(),
        number: 2,
        head_repo: RepoName::parse("platform-q-ai/quecto").unwrap(),
        branch: BranchName::parse("feat/2481-board_cli").unwrap(),
        pr: Some(2510),
        merged: Some(CommitId::parse("0123456789abcdef0123456789abcdef01234567").unwrap()),
        linked_by: person("Shane (work)", "shane@example.com"),
        unlinked: None,
        created: at("2026-10-11T12:00:00Z"),
        updated: at("2026-10-11T15:00:00Z"),
    }
}

fn round_trips(fields: ChangeFields) -> String {
    let change = Change::new(fields).unwrap();
    let file = encode::<ChangeCodec>(&change).unwrap();
    assert_eq!(decode::<ChangeCodec>(file.as_bytes()), Ok(change), "{file}");
    file
}

#[test]
fn the_example_change_file_is_byte_exact() {
    assert_eq!(round_trips(example()), EXAMPLE);
}

#[test]
fn linked_unlinked_and_merged_changes_round_trip() {
    let linked = ChangeFields {
        pr: None,
        merged: None,
        ..example()
    };
    let unlinked = ChangeFields {
        unlinked: Some((person("Bo", "bo@example.com"), at("2026-10-11T14:00:00Z"))),
        ..linked.clone()
    };
    let with_pr = ChangeFields {
        pr: Some(1),
        ..linked.clone()
    };
    let unlinked_file = round_trips(unlinked);
    assert!(
        unlinked_file.contains(
            "| Unlinked by | Bo (`bo@example.com`) |\n| Unlinked at | 2026-10-11T14:00:00Z |\n"
        ),
        "{unlinked_file}"
    );
    let linked_file = round_trips(linked);
    assert!(!linked_file.contains("Pull request") && !linked_file.contains("Merge commit"));
    round_trips(with_pr);
    round_trips(example());
}

#[test]
fn unlinked_by_without_unlinked_at_is_refused() {
    let linked = round_trips(ChangeFields {
        merged: None,
        ..example()
    });
    for row in [
        "| Unlinked by | Bo (`bo@example.com`) |\n",
        "| Unlinked at | 2026-10-11T14:00:00Z |\n",
    ] {
        let file = linked.replacen("| Created |", &format!("{row}| Created |"), 1);
        let read = decode::<ChangeCodec>(file.as_bytes());
        assert!(
            matches!(
                &read,
                Err(CodecError::Cell {
                    field: "Unlinked by",
                    ..
                })
            ),
            "{read:?}"
        );
    }
}

/// Git and the domain take a branch with `|`: a code cell holds it, the
/// `|` written `\|`, and a reader shows it as it is.
#[test]
fn a_branch_with_a_pipe_round_trips() {
    for (branch, written) in [("feat|x", "`feat\\|x`"), ("a|b|c", "`a\\|b\\|c`")] {
        let file = round_trips(ChangeFields {
            branch: BranchName::parse(branch).unwrap(),
            ..example()
        });
        assert!(
            file.contains(&format!("| Branch | {written} |\n")),
            "{file}"
        );
    }
}

/// A backtick no code span holds: refused when written, at its line.
#[test]
fn a_branch_with_a_backtick_is_refused() {
    let change = Change::new(ChangeFields {
        branch: BranchName::parse("feat`x").unwrap(),
        ..example()
    })
    .unwrap();
    assert_eq!(
        encode::<ChangeCodec>(&change),
        Err(CodecError::Markdown(MarkdownError::Cell {
            line: 9,
            why: "code of printable ASCII but a backtick"
        }))
    );
}

/// GitHub names repositories without regard to case; the file keeps the
/// case it was given, and reads it back.
#[test]
fn a_mixed_case_repository_keeps_its_case() {
    let file = round_trips(ChangeFields {
        head_repo: RepoName::parse("Platform-Q-AI/Quecto").unwrap(),
        ..example()
    });
    assert!(
        file.contains("| Repository | `Platform-Q-AI/Quecto` |\n"),
        "{file}"
    );
    let read = decode::<ChangeCodec>(file.as_bytes()).unwrap();
    assert_eq!(read.fields().head_repo.as_str(), "Platform-Q-AI/Quecto");
}

#[test]
fn a_repo_and_branch_with_underscores_round_trip() {
    for (repo, branch) in [
        ("_org_/__init__", "_x_/__y__"),
        ("org/www.site", "www.example/a@b"),
        ("my_org/my_repo", "feat/a<b>#1"),
    ] {
        let file = round_trips(ChangeFields {
            head_repo: RepoName::parse(repo).unwrap(),
            branch: BranchName::parse(branch).unwrap(),
            ..example()
        });
        assert!(
            file.contains(&format!("| Repository | `{repo}` |\n")),
            "{file}"
        );
    }
}

#[test]
fn the_widest_change_fits() {
    let identity = person(
        &"\u{1d538}".repeat(MAX_NAME_CHARS),
        // An email's `|` is written `\|`: the most bytes an email can take.
        &format!("{}@b", "|".repeat(MAX_EMAIL_BYTES - 2)),
    );
    let part = "a".repeat(MAX_REPO_PART_BYTES);
    let widest = ChangeFields {
        task: Slug::parse(&"a".repeat(64)).unwrap(),
        number: 999,
        head_repo: RepoName::parse(&format!("{part}/{part}")).unwrap(),
        // Every `|` written `\|`: the most bytes a branch can take.
        branch: BranchName::parse(&"|".repeat(MAX_BRANCH_BYTES)).unwrap(),
        pr: Some(u64::MAX),
        merged: None,
        linked_by: identity.clone(),
        unlinked: Some((identity, at("2026-10-11T14:00:00Z"))),
        ..example()
    };
    let merged = ChangeFields {
        unlinked: None,
        merged: Some(CommitId::parse(&"a".repeat(64)).unwrap()),
        ..widest.clone()
    };
    let mut most = 0;
    for fields in [widest, merged] {
        let file = round_trips(fields);
        let longest = file.lines().map(str::len).max().unwrap();
        most = most.max(file.len());
        assert!(file.len() <= CHANGE_LIMITS.file_bytes, "{}", file.len());
        assert!(file.lines().count() <= CHANGE_LIMITS.lines);
        assert!(
            longest <= CHANGE_LIMITS.line_bytes && longest > 900,
            "{longest}"
        );
    }
    // The unlinked one, its branch of `|`: 2,990 bytes.
    assert!(most >= 2_990, "{most}");
}

/// A file that spells a mixed-case repository in lower case reads back as
/// its own spelling: the value keeps the case the file has, so a case-only
/// change is a change.
#[test]
fn a_repository_reads_back_in_the_case_its_file_has() {
    let file = round_trips(ChangeFields {
        head_repo: RepoName::parse("Platform-Q-AI/Quecto").unwrap(),
        ..example()
    });
    let lower = file.replacen("`Platform-Q-AI/Quecto`", "`platform-q-ai/quecto`", 1);
    let read = decode::<ChangeCodec>(lower.as_bytes()).unwrap();
    assert_eq!(read.fields().head_repo.as_str(), "platform-q-ai/quecto");
    assert_eq!(encode::<ChangeCodec>(&read).unwrap(), lower);
}
