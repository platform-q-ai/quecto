//! Epic #2481 T1: a `board_op` record serializes as the event log reads
//! it, holds kinds, counts and a redacted slug only, and is never read
//! back.
use serde_json::{Value, json};

use super::*;
use crate::domain::audit::AuditEvent;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::RunOutcome;

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
        plans: None,
        runs: None,
        plan_version: None,
        items: None,
        roles: None,
        plan_state_from: None,
        plan_state_to: None,
        run: None,
        run_outcome: None,
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
        plans: Some(3),
        runs: Some(2),
        plan_version: Some(2),
        items: Some(5),
        roles: Some(2),
        plan_state_from: Some(StaticKind::new("draft")),
        plan_state_to: Some(StaticKind::new("accepted")),
        run: Some(1),
        run_outcome: Some(RunOutcomeKind::Failed),
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
            approach: Some(vec![TextChange::EdgeBlankLines]),
            items: Some(vec![TextChange::EdgeSpace]),
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
        "truncated_more": 10, "set_aside": 3, "set_aside_more": 1, "plans": 3, "runs": 2,
        "status_from": "in_progress", "status_to": "review",
        "claim": {"held_s": 5400, "left_s": -120, "holder_is_writer": false},
        "attempts": 2, "lost": 1, "pause_ms": 40, "completion": "failed",
        "completion_code": "rule", "completion_rule": "prs_unmerged", "prs_linked": 2,
        "prs_pending": 1, "completion_attempts": 2, "completion_lost": 1,
        "completion_pause_ms": 20,
        "cancelled": true,
        "build_us": 41_000, "result_bytes": 412, "duration_us": 1_830_000,
    });
    // A plan's counts and the normaliser's kinds (#2481 M4b), apart: one
    // macro holds only so many.
    let mut golden = golden;
    let plan = json!({
        "plan_version": 2, "items": 5, "roles": 2,
        "plan_state_from": "draft", "plan_state_to": "accepted", "run": 1, "run_outcome": "failed",
        "normalised": {
            "title": [], "description": ["crlf", "trailing_space"],
            "approach": ["edge_blank_lines"], "items": ["edge_space"],
        },
    });
    golden
        .as_object_mut()
        .unwrap()
        .extend(plan.as_object().unwrap().clone());
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
        "propose_plan",
        "edit_plan",
        "accept_plan",
        "discard_plan",
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

/// S4 (#2479): a start, as one `board_start` line holds it.
#[test]
fn a_start_serializes_as_its_golden_line() {
    let started = BoardStartRecord {
        task: TaskRef::of(&Slug::parse("demo-hello").unwrap()),
        client: Client::PocketRelay,
        outcome: OpOutcome::Ok,
        run: Some(1),
        plan_version: Some(2),
        members: Some(3),
        roles: Some(2),
        items: Some(4),
        not_applied: Some(1),
        claim: Some(StartClaim::Fresh),
        launch_us: Some(250_000),
        folded: Some(true),
        released: Some(false),
        duration_us: 900_000,
    };
    let line = serde_json::to_value(AuditEvent::BoardStart(Box::new(started))).unwrap();
    let golden = json!({
        "event": "board_start", "task": "demo-hello", "client": "pocket_relay",
        "outcome": "ok", "run": 1, "plan_version": 2, "members": 3, "roles": 2, "items": 4,
        "not_applied": 1, "claim": "fresh", "launch_us": 250_000, "folded": true,
        "released": false, "duration_us": 900_000,
    });
    assert_eq!(line, golden);
    let refused = BoardStartRecord {
        task: TaskRef::of(&Slug::parse("demo-hello").unwrap()),
        client: Client::Agent,
        outcome: OpOutcome::Refused {
            code: StaticKind::new("rule"),
            rule: Some(StaticKind::new("no_accepted_plan")),
        },
        run: None,
        plan_version: None,
        members: None,
        roles: None,
        items: None,
        not_applied: None,
        claim: None,
        launch_us: None,
        folded: None,
        released: None,
        duration_us: 40,
    };
    let line = serde_json::to_value(AuditEvent::BoardStart(Box::new(refused))).unwrap();
    let golden = json!({
        "event": "board_start", "task": "demo-hello", "client": "agent",
        "outcome": "refused", "code": "rule", "rule": "no_accepted_plan", "duration_us": 40,
    });
    assert_eq!(line, golden);
}

/// S4: renewals folded into one record, and a failure alone.
#[test]
fn claim_renewals_serialize_as_their_golden_lines() {
    let renewals = ClaimRenewalsRecord {
        task: TaskRef::of(&Slug::parse("demo-hello").unwrap()),
        run: 1,
        outcome: OpOutcome::Ok,
        renewals: 3,
        min_left_s: Some(2_640),
    };
    let line = serde_json::to_value(AuditEvent::BoardClaimRenewals(Box::new(renewals))).unwrap();
    let golden = json!({
        "event": "board_claim_renewals", "task": "demo-hello", "run": 1, "outcome": "ok",
        "renewals": 3, "min_left_s": 2_640,
    });
    assert_eq!(line, golden);
    let failed = ClaimRenewalsRecord {
        task: TaskRef::of(&Slug::parse("demo-hello").unwrap()),
        run: 1,
        outcome: OpOutcome::Refused {
            code: StaticKind::new("rule"),
            rule: Some(StaticKind::new("not_holder")),
        },
        renewals: 0,
        min_left_s: None,
    };
    let line = serde_json::to_value(AuditEvent::BoardClaimRenewals(Box::new(failed))).unwrap();
    let golden = json!({
        "event": "board_claim_renewals", "task": "demo-hello", "run": 1,
        "outcome": "refused", "code": "rule", "rule": "not_holder", "renewals": 0,
    });
    assert_eq!(line, golden);
}

/// S4: a fold-back, as one `board_fold_back` line holds it.
#[test]
fn a_fold_back_serializes_as_its_golden_line() {
    let folded = FoldBackRecord {
        task: TaskRef::of(&Slug::parse("demo-hello").unwrap()),
        run: 1,
        outcome: OpOutcome::Ok,
        run_outcome: Some(RunOutcomeKind::of(RunOutcome::Succeeded)),
        items: Some(ItemCounts {
            todo: 0,
            doing: 0,
            review: 1,
            done: 2,
            blocked: 0,
        }),
        unmatched: Some(0),
        extra: Some(1),
        evidence: Some(5),
        evidence_dropped: Some(1),
        cost_tokens: Some(812_000),
        attempts: Some(1),
        lost: Some(0),
        bytes: Some(1_900),
        duration_us: 120_000,
    };
    let line = serde_json::to_value(AuditEvent::BoardFoldBack(Box::new(folded))).unwrap();
    let golden = json!({
        "event": "board_fold_back", "task": "demo-hello", "run": 1, "outcome": "ok",
        "run_outcome": "succeeded",
        "items": {"todo": 0, "doing": 0, "review": 1, "done": 2, "blocked": 0},
        "unmatched": 0, "extra": 1, "evidence": 5, "evidence_dropped": 1,
        "cost_tokens": 812_000, "attempts": 1, "lost": 0, "bytes": 1_900,
        "duration_us": 120_000,
    });
    assert_eq!(line, golden);
    let bare = FoldBackRecord {
        task: TaskRef::of(&Slug::parse("demo-hello").unwrap()),
        run: 1,
        outcome: OpOutcome::Refused {
            code: StaticKind::new("contended"),
            rule: None,
        },
        run_outcome: None,
        items: None,
        unmatched: None,
        extra: None,
        evidence: None,
        evidence_dropped: None,
        cost_tokens: None,
        attempts: None,
        lost: None,
        bytes: None,
        duration_us: 9,
    };
    let line = serde_json::to_value(AuditEvent::BoardFoldBack(Box::new(bare))).unwrap();
    let golden = json!({
        "event": "board_fold_back", "task": "demo-hello", "run": 1, "outcome": "refused",
        "code": "contended", "duration_us": 9,
    });
    assert_eq!(line, golden);
}

/// S4: the start's claim kinds, and every run outcome a fold-back names,
/// are known kinds; none of the S4 lines is read back.
#[test]
fn the_s4_kinds_are_frozen_and_known() {
    let claims: Vec<&str> = StartClaim::ALL.iter().map(|kind| kind.as_str()).collect();
    assert_eq!(claims, ["fresh", "held"]);
    for kind in StartClaim::ALL {
        assert_eq!(
            serde_json::to_value(kind).unwrap(),
            Value::from(kind.as_str())
        );
        assert_eq!(StaticKind::new(kind.as_str()).as_str(), kind.as_str());
    }
    // Each run outcome has its kind, of its own name.
    for outcome in RunOutcome::ALL {
        let kind = RunOutcomeKind::of(*outcome);
        assert_eq!(kind.as_str(), outcome.as_str());
        assert_eq!(StaticKind::new(kind.as_str()).as_str(), kind.as_str());
    }
    assert_eq!(RunOutcomeKind::ALL.len(), RunOutcome::ALL.len());
    let line = json!({"event": "board_start", "task": "t", "client": "agent", "outcome": "ok", "duration_us": 0});
    assert!(serde_json::from_value::<AuditEvent>(line).is_err());
}
