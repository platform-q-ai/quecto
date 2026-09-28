//! Decode → project over the captured streams (#2285 review), compared
//! with the values in the captures. Infrastructure may name the
//! application here: this is test code.

use super::super::*;
use super::SAMPLES;
use crate::application::external_agent::dto::{ExecutionState, MessageRole, TurnOutcome};
use crate::application::external_agent::projection::Projector;
use crate::domain::external_agent::turn::TurnEnd;

/// Project a sample; `user_turns` are recorded before the given line
/// indexes (the session records what it writes to stdin).
fn project(name: &str, user_turns: &[(usize, &str)]) -> (Projector, Vec<TurnOutcome>) {
    let (_, text) = SAMPLES
        .iter()
        .find(|(sample, _)| *sample == name)
        .expect("a known sample");
    let mut decoder = StreamJsonDecoder::new();
    let mut projector = Projector::new();
    let mut ends = Vec::new();
    for (index, line) in text.lines().enumerate() {
        for (_, turn) in user_turns.iter().filter(|(at, _)| *at == index) {
            projector.record_user_turn(turn);
        }
        for event in decoder.decode_line(line).expect("decodes") {
            ends.extend(projector.apply(&event).turn_end);
        }
    }
    (projector, ends)
}

fn costs(ends: &[TurnOutcome]) -> Vec<u64> {
    ends.iter().map(|end| end.usage.cost_micro_usd).collect()
}

#[test]
fn rt_projects_two_completed_turns_with_delta_costs() {
    let (projector, ends) = project("rt", &[(0, "work the board"), (30, "what task id?")]);
    assert_eq!(costs(&ends), [44_864, 8_201]);
    assert!(ends.iter().all(|end| end.end == TurnEnd::Completed));
    let totals = projector.session_totals();
    assert_eq!(totals.cost_micro_usd, 53_065);
    assert_eq!((totals.tokens.input, totals.tokens.output), (84, 1431));
    assert_eq!(totals.user_messages, 2);
    assert_eq!(totals.tool_calls, 9);
    assert_eq!(totals.tool_results, 9);
    let report = projector.report().expect("a report");
    assert!(
        report
            .content
            .ends_with("**The task id I submitted earlier was T1.**")
    );
    assert_eq!(projector.state(), ExecutionState::Idle);
}

#[test]
fn guards_projects_both_refusals_as_tool_errors_and_audit_entries() {
    let (projector, ends) = project("guards", &[(0, "write notes.txt"), (9, "git push")]);
    assert_eq!(costs(&ends), [15_312, 5_097]);
    assert!(ends.iter().all(|end| end.end == TurnEnd::Completed));
    let refused: Vec<_> = projector
        .messages()
        .iter()
        .filter(|m| m.role == MessageRole::Tool)
        .collect();
    assert_eq!(refused.len(), 2);
    assert!(refused.iter().all(|m| m.is_error && m.permission_denied));
    let audit: Vec<_> = projector
        .guardrail_audit()
        .map(|d| (d.tool_name.as_str(), d.tool_use_id.as_str(), d.turn))
        .collect();
    assert_eq!(
        audit,
        [
            ("Write", "toolu_01PkECDLU8eoNSBHxva5ghcA", 0),
            ("Bash", "toolu_01C1BieTLDzgUouWDEZRiNff", 1),
        ]
    );
    assert_eq!(
        projector.report().map(|r| r.content.as_str()),
        Some("quecto: command refused by the swarm denylist ('git push')")
    );
}

#[test]
fn mid_projects_one_turn_end_for_both_inputs() {
    // The wake was written while the Bash tool ran: before line 8.
    let (projector, ends) = project(
        "mid",
        &[(0, "sleep 15 && echo slept"), (8, "new board message")],
    );
    assert_eq!(ends.len(), 1);
    assert_eq!(costs(&ends), [18_512]);
    let users: Vec<_> = projector
        .messages()
        .iter()
        .filter(|m| m.role == MessageRole::User)
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(users, ["sleep 15 && echo slept", "new board message"]);
    assert_eq!(
        projector.report().map(|r| r.content.as_str()),
        Some("Done — I've acknowledged the board message.")
    );
}

#[test]
fn kill_projects_the_background_sleep() {
    let (projector, ends) = project("kill", &[(0, "sleep 300 in the background")]);
    assert_eq!(ends.len(), 1);
    let jobs: Vec<_> = projector.background_jobs().collect();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].task_id, "bup00mo5i");
    assert!(jobs[0].is_backgrounded);
    assert_eq!(jobs[0].description.as_deref(), Some("sleep 300"));
}

#[test]
fn error_results_project_failed_turns_that_report_their_errors() {
    let (projector, ends) = project("errors", &[(0, "loop"), (3, "spend"), (5, "crash")]);
    assert_eq!(costs(&ends), [12_000, 498_000, 5_000]);
    let failures: Vec<_> = ends
        .iter()
        .map(|end| match &end.end {
            TurnEnd::Failed(failure) => failure.clone(),
            TurnEnd::Completed => panic!("an error result fails the turn"),
        })
        .collect();
    assert_eq!(failures[0].terminal_reason.as_deref(), Some("max_turns"));
    assert_eq!(
        failures[1].terminal_reason.as_deref(),
        Some("budget_exhausted")
    );
    assert_eq!(failures[2].terminal_reason, None);
    let report = projector.report().expect("a report");
    assert!(
        report.content.contains("API Error: 529 Overloaded"),
        "{}",
        report.content
    );
    assert_eq!(report.failure.as_ref(), Some(&failures[2]));
}
