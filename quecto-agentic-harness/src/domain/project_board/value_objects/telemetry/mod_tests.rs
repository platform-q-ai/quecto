//! Epic #2481 T1: a `board_op` record serializes as the event log reads
//! it, holds kinds, counts and a redacted slug only, and is never read
//! back.
use serde_json::{Value, json};

use super::*;
use crate::domain::audit::AuditEvent;
use crate::domain::project_board::value_objects::slug::Slug;

const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

fn bare() -> BoardOpRecord {
    BoardOpRecord {
        op: OpKind::Show,
        front_end: FrontEnd::Cli,
        client: Client::PocketRelay,
        access: Access::Read,
        outcome: OpOutcome::Ok,
        fault: None,
        task: None,
        pr: None,
        head: None,
        listing: None,
        status_from: None,
        status_to: None,
        claim: None,
        attempts: None,
        lost: None,
        pause_ms: None,
        completion: None,
        normalised: None,
        cancelled: false,
        build_us: None,
        result_bytes: 0,
        duration_us: 0,
    }
}

/// Every field set, as one `board_op` line holds it.
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
            unexpected_count: Some(2),
        }),
        task: Some(TaskRef::of(&Slug::parse("fix-index-rebuild").unwrap())),
        pr: Some(7),
        head: CommitId::parse(HEAD).ok(),
        listing: Some(ListingCounts {
            listed: 40,
            shown: 30,
            truncated_more: 10,
            set_aside: 3,
            set_aside_more: 1,
        }),
        status_from: Some(StaticKind::new("in_progress")),
        status_to: Some(StaticKind::new("review")),
        claim: Some(ClaimFacts {
            held_s: 5400,
            left_s: -120,
            holder_is_writer: false,
        }),
        attempts: Some(2),
        lost: Some(1),
        pause_ms: Some(40),
        completion: Some(CompletionFacts {
            completion: CompletionKind::Failed,
            completion_code: Some(StaticKind::new("rule")),
            completion_rule: Some(StaticKind::new("prs_unmerged")),
            prs_linked: 2,
            prs_pending: Some(1),
            completion_attempts: Some(2),
            completion_lost: Some(1),
            completion_pause_ms: Some(20),
        }),
        normalised: Some(Normalised {
            title: Some(vec![]),
            description: Some(vec![TextChange::Crlf, TextChange::TrailingSpace]),
        }),
        cancelled: true,
        build_us: Some(41_000),
        result_bytes: 412,
        duration_us: 1_830_000,
    }
}

#[test]
fn every_field_serializes_as_its_golden_line() {
    let line = serde_json::to_value(AuditEvent::BoardOp(Box::new(full()))).unwrap();
    let golden = json!({
        "event": "board_op", "op": "move", "front_end": "tool", "client": "agent",
        "access": "write", "outcome": "refused", "code": "rule", "rule": "not_holder",
        "field": "other", "fault": "unexpected", "unexpected_count": 2,
        "task": "fix-index-rebuild", "pr": 7, "head": HEAD, "listed": 40, "shown": 30,
        "truncated_more": 10, "set_aside": 3, "set_aside_more": 1,
        "status_from": "in_progress", "status_to": "review",
        "claim": {"held_s": 5400, "left_s": -120, "holder_is_writer": false},
        "attempts": 2, "lost": 1, "pause_ms": 40, "completion": "failed",
        "completion_code": "rule", "completion_rule": "prs_unmerged", "prs_linked": 2,
        "prs_pending": 1, "completion_attempts": 2, "completion_lost": 1,
        "completion_pause_ms": 20,
        "normalised": {"title": [], "description": ["crlf", "trailing_space"]},
        "cancelled": true,
        "build_us": 41_000, "result_bytes": 412, "duration_us": 1_830_000,
    });
    assert_eq!(line, golden);
}

#[test]
fn a_bare_record_writes_only_what_always_applies() {
    let line = serde_json::to_value(AuditEvent::BoardOp(Box::new(bare()))).unwrap();
    let golden = json!({
        "event": "board_op", "op": "show", "front_end": "cli", "client": "pocket_relay",
        "access": "read", "outcome": "ok", "result_bytes": 0, "duration_us": 0,
    });
    assert_eq!(line, golden);
}

#[test]
fn drops_are_noted_as_their_own_event() {
    let dropped = AuditEvent::BoardOpsDropped { dropped: 3 };
    let line = serde_json::to_value(&dropped).unwrap();
    assert_eq!(line, json!({"event": "board_ops_dropped", "dropped": 3}));
    assert_eq!(serde_json::from_value::<AuditEvent>(line).unwrap(), dropped);
}

/// #2532 round-2 review M2: nothing reads a `board_op` line back; a
/// reader (T8) brings its own reader-side type.
#[test]
fn a_board_op_line_is_never_read_back() {
    for record in [full(), bare()] {
        let line = serde_json::to_value(AuditEvent::BoardOp(Box::new(record))).unwrap();
        assert!(serde_json::from_value::<AuditEvent>(line).is_err());
    }
}

/// Each kind's every name as it has been given (append-only): a kind may
/// be added, never renamed or taken away, so every name here must still
/// be one, and each kind serializes as its name, shaped as a kind.
#[test]
fn no_kind_is_ever_renamed_or_removed() {
    fn frozen<T: Copy + Serialize>(all: &[T], name: impl Fn(T) -> &'static str, names: &[&str]) {
        let current: Vec<&str> = all.iter().map(|kind| name(*kind)).collect();
        for kept in names {
            assert!(
                current.contains(kept),
                "the kind {kept} was renamed or removed"
            );
        }
        for kind in all {
            let serialized = serde_json::to_value(kind).unwrap();
            assert_eq!(serialized, Value::from(name(*kind)));
            assert_eq!(StaticKind::new(name(*kind)).as_str(), name(*kind));
        }
    }
    let ops = [
        "bootstrap",
        "list",
        "list_ready",
        "list_takeable",
        "show",
        "create",
        "update_draft",
        "move",
        "renew",
        "take_over",
        "link_pr",
        "unlink_pr",
        "complete",
        "clear_set_aside",
        "unknown",
    ];
    frozen(OpKind::ALL, OpKind::as_str, &ops);
    frozen(FrontEnd::ALL, FrontEnd::as_str, &["tool", "cli"]);
    let clients = ["agent", "pocket_relay", "cli", "other"];
    frozen(Client::ALL, Client::as_str, &clients);
    frozen(Access::ALL, Access::as_str, &["read", "write"]);
    let faults = [
        "missing",
        "unexpected",
        "wrong_type",
        "invalid_value",
        "too_large",
        "not_json",
        "duplicate_key",
    ];
    frozen(FaultKind::ALL, FaultKind::as_str, &faults);
    let completions = ["done", "pending", "failed"];
    frozen(CompletionKind::ALL, CompletionKind::as_str, &completions);
    let changes = [
        "crlf",
        "lone_cr",
        "line_break",
        "chat_space",
        "tab",
        "edge_blank_lines",
        "trailing_space",
        "edge_space",
    ];
    frozen(TextChange::ALL, TextChange::as_str, &changes);
}

#[test]
fn an_op_name_finds_its_kind_and_any_other_text_is_unknown() {
    for kind in OpKind::ALL.iter().filter(|kind| **kind != OpKind::Unknown) {
        assert_eq!(OpKind::named(kind.as_str()), *kind);
    }
    for text in ["unknown", "Move", "move ", "", "drop table"] {
        assert_eq!(OpKind::named(text), OpKind::Unknown, "{text:?}");
    }
}
