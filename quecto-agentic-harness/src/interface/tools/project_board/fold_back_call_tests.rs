//! S4: one `board_fold_back` record per recorded fold-back, holding its
//! outcome, codes and counts, never text; none while unrecorded.
use std::sync::Arc;

use super::*;
use crate::application::project_board::fold_back::{SwarmEnd, SwarmRunView};
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::use_cases::FoldBackRun;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::telemetry::RunOutcomeKind;
use crate::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use crate::interface::tools::project_board::recording::tests::{Records, recording};

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

#[test]
fn a_recorded_fold_back_writes_one_record_of_its_kinds_and_counts() {
    let records = Arc::new(Records::default());
    let mut call = FoldBackCall::start(Some(recording(&records)));
    let facts = call.facts().expect("a recorded call gathers facts");
    facts.run_outcome = Some(RunOutcomeKind::Failed);
    facts.evidence_dropped = Some(2);
    facts.attempts = Some(1);
    let folded = Ok(FoldedBack::Ended(3));
    call.finish(&slug("board-store"), 3, &folded);
    let written = records.fold_backs();
    assert_eq!(written.len(), 1);
    let line = serde_json::to_value(&written[0]).unwrap();
    assert_eq!(line["outcome"], "ok", "{line}");
    assert_eq!(line["run"], 3, "{line}");
    assert_eq!(line["run_outcome"], "failed", "{line}");
    assert_eq!(line["evidence_dropped"], 2, "{line}");
    assert!(line.get("items").is_none(), "{line}");
    assert!(records.taken().is_empty(), "no board_op is written");
}

#[test]
fn a_refused_fold_back_is_recorded_with_its_code_and_rule() {
    let records = Arc::new(Records::default());
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let use_case = FoldBackRun::new(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(MarkdownTextCells),
    )
    .unwrap();
    let view = SwarmRunView {
        end: SwarmEnd::Succeeded,
        items: vec![],
        observed_tokens: 9,
    };
    // The board holds no such task: refused with its code, and no rule.
    let mut call = FoldBackCall::start(Some(recording(&records)));
    let result = use_case.execute_noted(&slug("board-store"), 1, &view, call.facts());
    call.finish(&slug("board-store"), 1, &result);
    let line = serde_json::to_value(&records.fold_backs()[0]).unwrap();
    assert_eq!(line["outcome"], "refused", "{line}");
    assert_eq!(line["code"], "not_found", "{line}");
    assert!(line.get("rule").is_none(), "{line}");
    assert!(line.get("run_outcome").is_none(), "{line}");
    // A rule's refusal names the rule.
    let call = FoldBackCall::start(Some(recording(&records)));
    let refused = Err(ProjectBoardError::Rule(BoardRuleError::NoSuchRun(4)));
    call.finish(&slug("board-store"), 4, &refused);
    let line = serde_json::to_value(&records.fold_backs()[0]).unwrap();
    assert_eq!(
        (&line["code"], &line["rule"]),
        (&"rule".into(), &"no_such_run".into())
    );
}

#[test]
fn an_unrecorded_fold_back_gathers_nothing_and_writes_nothing() {
    let mut call = FoldBackCall::start(None);
    assert!(call.facts().is_none());
    call.finish(&slug("board-store"), 1, &Ok(FoldedBack::Ended(1)));
}
