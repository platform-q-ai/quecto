//! Epic #2481 T3: the project board's telemetry is documented field by
//! field (`docs/project-board.md#telemetry`), and the README points there.
use std::collections::BTreeSet;

use quecto::domain::audit::AuditEvent;
use quecto::domain::project_board::value_objects::code_ref::CommitId;
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::telemetry::{
    Access, BoardOpRecord, ClaimFacts, Client, CompletionFacts, CompletionKind, FaultKind,
    FrontEnd, ListingCounts, Normalised, OpKind, OpOutcome, RequestFault, StaticKind, TaskRef,
    TextChange,
};

use crate::common::read_repo_file;

/// The telemetry section of `docs/project-board.md`.
fn telemetry_doc() -> String {
    let doc = read_repo_file("docs/project-board.md");
    let (_, section) = doc
        .split_once("\n## Telemetry\n")
        .expect("docs/project-board.md has a Telemetry section");
    section.to_owned()
}

/// A record with every field set.
fn full() -> BoardOpRecord {
    BoardOpRecord {
        op: OpKind::Move,
        front_end: FrontEnd::Tool,
        client: Client::Agent,
        access: Access::Write,
        outcome: OpOutcome::Refused {
            code: StaticKind::new("rule"),
            rule: Some(StaticKind::new("not_holder")),
        },
        fault: Some(RequestFault {
            field: StaticKind::OTHER,
            fault: FaultKind::Unexpected,
            unexpected_count: Some(1),
        }),
        task: Some(TaskRef::of(&Slug::parse("a").unwrap())),
        pr: Some(7),
        head: CommitId::parse("0123456789abcdef0123456789abcdef01234567").ok(),
        listing: Some(ListingCounts {
            listed: 2,
            shown: 1,
            truncated_more: 1,
            set_aside: 0,
            set_aside_more: 0,
        }),
        plans: Some(3),
        runs: Some(2),
        plan_version: Some(2),
        items: Some(5),
        roles: Some(2),
        plan_state_from: Some(StaticKind::new("draft")),
        plan_state_to: Some(StaticKind::new("accepted")),
        status_from: Some(StaticKind::new("in_progress")),
        status_to: Some(StaticKind::new("review")),
        claim: Some(ClaimFacts {
            held_s: 1,
            left_s: -1,
            holder_is_writer: true,
        }),
        attempts: Some(2),
        lost: Some(1),
        pause_ms: Some(1),
        completion: Some(CompletionFacts {
            completion: CompletionKind::Failed,
            completion_code: Some(StaticKind::new("rule")),
            completion_rule: Some(StaticKind::new("no_prs")),
            prs_linked: 1,
            prs_pending: Some(1),
            completion_attempts: Some(1),
            completion_lost: Some(0),
            completion_pause_ms: Some(1),
        }),
        normalised: Some(Normalised {
            title: Some(vec![TextChange::Crlf]),
            description: Some(vec![]),
            approach: Some(vec![TextChange::EdgeBlankLines]),
            items: Some(vec![TextChange::EdgeSpace]),
        }),
        cancelled: true,
        build_us: Some(1),
        result_bytes: 1,
        duration_us: 1,
    }
}

/// The names in backticks in the first cell of each row of the field
/// table.
fn documented_fields(section: &str) -> BTreeSet<String> {
    section
        .lines()
        .filter(|line| line.starts_with("| `"))
        .flat_map(|line| {
            let cell = line.split('|').nth(1).unwrap_or_default();
            cell.split('`')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn every_field_a_board_op_record_writes_is_documented_and_no_other() {
    let line = serde_json::to_value(AuditEvent::BoardOp(Box::new(full()))).unwrap();
    let mut written: BTreeSet<String> = line.as_object().unwrap().keys().cloned().collect();
    written.remove("event");
    // The envelope's fields the page explains for the board.
    written.extend(["session".to_string(), "turn".to_string()]);
    assert_eq!(documented_fields(&telemetry_doc()), written);
}

#[test]
fn every_kind_a_front_end_records_is_named() {
    let section = telemetry_doc();
    let kinds = OpKind::ALL.iter().map(|kind| kind.as_str());
    let kinds = kinds
        .chain(FrontEnd::ALL.iter().map(|kind| kind.as_str()))
        .chain(Client::ALL.iter().map(|kind| kind.as_str()))
        .chain(Access::ALL.iter().map(|kind| kind.as_str()))
        .chain(FaultKind::ALL.iter().map(|kind| kind.as_str()))
        .chain(CompletionKind::ALL.iter().map(|kind| kind.as_str()))
        .chain(TextChange::ALL.iter().map(|kind| kind.as_str()));
    for kind in kinds {
        assert!(
            section.contains(&format!("`{kind}`")),
            "`{kind}` is not named"
        );
    }
    for told in [
        "QUECTO_BOARD_CLIENT",
        "`pocket-relay`",
        "board-cli_<UTC date>.jsonl",
        "board_ops_dropped",
        "quecto::project_board",
        "board-cli:<UTC date>",
        "`internal`",
    ] {
        assert!(section.contains(told), "{told} is not told");
    }
}

#[test]
fn the_readme_points_to_the_board_telemetry() {
    let readme = read_repo_file("README.md");
    assert!(readme.contains("docs/project-board.md#telemetry"));
    assert!(readme.contains("audit/board-cli_<UTC date>.jsonl"));
}
