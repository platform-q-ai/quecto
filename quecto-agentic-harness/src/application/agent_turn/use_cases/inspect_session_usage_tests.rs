//! #1855: a session's usage is retained across its turns and reported with
//! its history's message counts and context occupancy.
use super::*;
use crate::domain::conversation::value_objects::message::ToolCall;

fn turn(input: u64, output: u64, read: u64, write: u64, cost: u64) -> UsageTotals {
    UsageTotals::billed(input, output, read, write, cost)
}

#[test]
fn a_session_that_used_nothing_reports_its_history_only() {
    let report = InspectSessionUsage::history_report("cli:test", &[]);
    assert_eq!(report.session_key, "cli:test");
    assert_eq!(report.messages, MessageCounts::default());
    assert_eq!(report.usage, SessionUsage::default());
    assert_eq!(report.cost_usd, 0.0);
    assert_eq!(report.cache_hit_ratio, None);
    assert_eq!((report.context_tokens, report.max_context_tokens), (0, 0));
}

#[test]
fn the_report_counts_each_kind_of_message() {
    let call = |id: &str| ToolCall {
        id: id.into(),
        name: "bash".into(),
        arguments: "{}".into(),
    };
    let messages = vec![
        Message::system("prompt"),
        Message::user("hi"),
        Message::assistant(String::new(), vec![call("c1"), call("c2")]),
        Message::tool("c1".to_string(), "out1".to_string()),
        Message::tool("c2".to_string(), "out2".to_string()),
        Message::assistant("done", vec![]),
        Message::user("bye"),
    ];
    let report = InspectSessionUsage::history_report("k", &messages);
    assert_eq!(
        report.messages,
        MessageCounts {
            user: 2,
            assistant: 2,
            tool_calls: 2,
            tool_results: 2,
            total: 7,
        }
    );
}

#[test]
fn usage_is_retained_across_turns() {
    let mut usage = InspectSessionUsage::new();
    usage.record_turn("cli:test", turn(10, 2, 3, 4, 1_000), 12);
    usage.record_turn("cli:test", turn(20, 5, 6, 7, 2_500), 25);
    let retained = usage.usage();
    assert_eq!(
        retained.tokens,
        crate::application::agent_turn::dto::session_usage::TokenTotals {
            input: 30,
            output: 7,
            cache_read: 9,
            cache_write: 11,
            total: 37,
        }
    );
    assert_eq!(retained.cost_micro_usd, 3_500);
    assert!((retained.cost_usd() - 0.0035).abs() < f64::EPSILON);
}

#[test]
fn the_report_pairs_the_retained_usage_with_the_context_occupancy() {
    let mut usage = InspectSessionUsage::new();
    usage.record_turn("cli:test", turn(100, 25, 10, 5, 12_300), 42);
    let messages = vec![Message::user("hi"), Message::assistant("reply", vec![])];
    let report = usage.report("k", &messages, 42, 1_000);
    assert_eq!(report.usage.tokens.total, 125);
    assert_eq!(report.usage.cost_micro_usd, 12_300);
    assert!((report.cost_usd - 0.0123).abs() < f64::EPSILON);
    assert!((report.cache_hit_ratio.unwrap() - (10.0 / 115.0)).abs() < 1e-9);
    assert_eq!(
        (report.context_tokens, report.max_context_tokens),
        (42, 1_000)
    );
    assert_eq!(report.messages.total, 2);
}

#[test]
fn totals_saturate_rather_than_wrap() {
    let mut usage = InspectSessionUsage::new();
    usage.record_turn("k", turn(u64::MAX, 1, u64::MAX, 0, u64::MAX), 0);
    usage.record_turn("k", turn(1, 1, 1, 0, 1), 0);
    let retained = usage.usage();
    assert_eq!(retained.tokens.input, u64::MAX);
    assert_eq!(retained.tokens.cache_read, u64::MAX);
    assert_eq!(retained.tokens.total, u64::MAX);
    assert_eq!(retained.cost_micro_usd, u64::MAX);
}

#[test]
fn diagnostics_are_merged_and_a_clear_drops_everything() {
    let mut usage = InspectSessionUsage::new();
    usage.record_turn("k", turn(10, 2, 0, 0, 5), 0);
    let diagnostics = RequestDiagnostics {
        logical_requests: 3,
        ..RequestDiagnostics::default()
    };
    usage.record_request_diagnostics(diagnostics.clone());
    usage.record_request_diagnostics(diagnostics);
    assert_eq!(usage.usage().request_diagnostics.logical_requests, 6);
    usage.clear();
    assert_eq!(usage.usage(), &SessionUsage::default());
}

/// #1855 review L1: a session whose billed usage was never priced reports
/// an unknown cost, not a zero one; once any of it is priced the known part
/// is reported with the unpriced count beside it. Nothing used costs zero.
#[test]
fn unpriced_usage_is_reported_unknown_never_free() {
    let unpriced = UsageTotals {
        unpriced_requests: 1,
        ..turn(10, 2, 0, 0, 0)
    };
    let mut usage = InspectSessionUsage::new();
    assert_eq!(usage.report("k", &[], 0, 0).cost_micro_usd, Some(0));
    usage.record_turn("k", unpriced, 0);
    let report = usage.report("k", &[], 0, 0);
    assert_eq!(report.cost_micro_usd, None, "unpriced is not free");
    assert_eq!(report.usage.unpriced_requests, 1);
    usage.record_turn("k", turn(10, 2, 0, 0, 40), 0);
    let report = usage.report("k", &[], 0, 0);
    assert_eq!(report.cost_micro_usd, Some(40));
    assert_eq!(
        report.usage.unpriced_requests, 1,
        "the unpriced part is said"
    );
}
