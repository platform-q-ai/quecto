//! S4: one `board_fold_back` record per recorded fold-back, holding its
//! outcome, codes and counts, never text.
use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::fold_back::{SwarmEnd, SwarmRunView};
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::telemetry::FoldBackFacts;
use crate::application::project_board::use_cases::FoldBackRun;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::RunOutcomeKind;
use crate::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use crate::interface::tools::project_board::recording::tests::{Records, recording};

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn recorded(
    records: &Arc<Records>,
    result: &Result<FoldedBack, ProjectBoardError>,
    facts: &FoldBackFacts,
) -> serde_json::Value {
    FoldBackCall::new(recording(records)).record(FoldBackOutcome {
        task: &slug("board-store"),
        run: 3,
        result,
        facts,
        elapsed: Duration::from_micros(42),
    });
    let written = records.fold_backs();
    assert_eq!(written.len(), 1, "{written:?}");
    assert!(records.taken().is_empty(), "no board_op is written");
    serde_json::to_value(&written[0]).unwrap()
}

#[test]
fn a_recorded_fold_back_writes_one_record_of_its_kinds_and_counts() {
    let records = Arc::new(Records::default());
    let facts = FoldBackFacts {
        run_outcome: Some(RunOutcomeKind::Failed),
        evidence_dropped: Some(2),
        attempts: Some(1),
        ..FoldBackFacts::default()
    };
    let line = recorded(&records, &Ok(FoldedBack::Ended(3)), &facts);
    assert_eq!(line["outcome"], "ok", "{line}");
    assert_eq!(line["run"], 3, "{line}");
    assert_eq!(line["run_outcome"], "failed", "{line}");
    assert_eq!(line["evidence_dropped"], 2, "{line}");
    assert_eq!(line["duration_us"], 42, "{line}");
    assert!(line.get("items").is_none(), "{line}");
}

#[test]
fn a_refused_fold_back_is_recorded_with_its_code_and_rule() {
    let records = Arc::new(Records::default());
    let refused = Err(ProjectBoardError::Rule(BoardRuleError::NoSuchRun(4)));
    let line = recorded(&records, &refused, &FoldBackFacts::default());
    assert_eq!(
        (&line["outcome"], &line["code"], &line["rule"]),
        (&"refused".into(), &"rule".into(), &"no_such_run".into())
    );
    // Through the use case: the board holds no such task.
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
    let call = FoldBackCall::new(recording(&records));
    let result = use_case.execute_recorded(&slug("board-store"), 1, &view, Some(&call));
    assert_eq!(
        result,
        Err(ProjectBoardError::NotFound(slug("board-store")))
    );
    let line = serde_json::to_value(&records.fold_backs()[0]).unwrap();
    assert_eq!(line["outcome"], "refused", "{line}");
    assert_eq!(line["code"], "not_found", "{line}");
    assert!(line.get("rule").is_none(), "{line}");
    assert!(line.get("run_outcome").is_none(), "{line}");
}
