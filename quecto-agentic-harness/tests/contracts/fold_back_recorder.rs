//! Contract for [`FoldBackRecorder`] (S4): the interface's adapter writes
//! each outcome it is given as one `board_fold_back` line of the event
//! log, under no turn, with the answer's error and rule codes and the
//! facts' kinds and counts, never text.
use std::sync::Arc;
use std::time::Duration;

use quecto::application::project_board::dto::FoldBackFacts;
use quecto::application::project_board::dto::FoldedBack;
use quecto::application::project_board::dto::ProjectBoardError;
use quecto::application::project_board::ports::{FoldBackOutcome, FoldBackRecorder};
use quecto::composition::bootstrap::project_board::session_board_recording_decided;
use quecto::domain::project_board::value_objects::rule_error::BoardRuleError;
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::domain::project_board::value_objects::telemetry::RunOutcomeKind;
use quecto::infrastructure::observability::audit_log::AuditLog;
use quecto::interface::tools::project_board::fold_back_call::FoldBackCall;
use serde_json::Value;

#[test]
fn each_outcome_is_one_fold_back_line_of_codes_and_counts() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:recorder").unwrap();
    let recording = session_board_recording_decided(true, &log).expect("a recording while on");
    let recorder = FoldBackCall::new(Arc::new(recording));
    let task = Slug::parse("board-store").unwrap();
    let facts = FoldBackFacts {
        run_outcome: Some(RunOutcomeKind::Succeeded),
        evidence: Some(2),
        attempts: Some(1),
        ..FoldBackFacts::default()
    };
    let refused = Err(ProjectBoardError::Rule(BoardRuleError::NoSuchRun(2)));
    for result in [Ok(FoldedBack::Ended(1)), refused] {
        recorder.record(FoldBackOutcome {
            task: &task,
            run: 1,
            result: &result,
            facts: &facts,
            elapsed: Duration::from_micros(7),
        });
    }
    let text = std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:recorder")).unwrap();
    let lines: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    for line in &lines {
        assert_eq!(line["event"], "board_fold_back");
        assert_eq!(line["turn"], Value::Null);
        assert_eq!(line["run_outcome"], "succeeded");
        assert_eq!(line["evidence"], 2);
        assert_eq!(line["duration_us"], 7);
    }
    assert_eq!(lines[0]["outcome"], "ok");
    assert_eq!(
        (&lines[1]["outcome"], &lines[1]["code"], &lines[1]["rule"]),
        (&"refused".into(), &"rule".into(), &"no_such_run".into())
    );
}
