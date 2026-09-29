//! #2313 review M1 and L3: the run-wide section holds the board's totals
//! for every member, bounded, and the run's wall time from its creation.
use super::*;

fn usage(member: &str, requests: u64) -> MemberUsage {
    MemberUsage {
        actor_ref: member.into(),
        requests,
        tokens: 10 * requests,
        unknown_usage_requests: 0,
        attempts: requests,
        input_tokens: 7 * requests,
        output_tokens: 3 * requests,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
    }
}

/// The wall time runs from the run's creation to the board's read, in
/// microseconds; a creation time the board does not hold, or one after
/// the read, gives none or zero, never a negative span.
#[test]
fn the_wall_time_runs_from_the_runs_creation_to_the_read() {
    let totals = |created| {
        RunTotals::new(
            TaskStates::default(),
            MessageTotals::default(),
            Vec::new(),
            created,
            1_000.25,
        )
        .wall_time_us
    };
    assert_eq!(totals(Some(990.0)), Some(10_250_000));
    assert_eq!(totals(None), None);
    assert_eq!(totals(Some(f64::NAN)), None);
    assert_eq!(totals(Some(2_000.0)), Some(0), "never negative");
}

/// Past [`REQUEST_MEMBERS`] members, the rest are counted, not listed.
#[test]
fn the_usage_is_bounded_and_the_rest_counted() {
    let members: Vec<_> = (0..REQUEST_MEMBERS + 2)
        .map(|n| usage(&format!("m{n}"), 1))
        .collect();
    let totals = RunTotals::new(
        TaskStates::default(),
        MessageTotals::default(),
        members,
        None,
        0.0,
    );
    assert_eq!(totals.usage.len(), REQUEST_MEMBERS);
    assert_eq!(totals.unlisted_usage, 2);
    assert_eq!(totals.usage[0].actor_ref.as_str(), "m0", "the board's order");
}

/// The section is counts and redacted refs only, and reads back as it
/// was written; `unlisted_usage` is left out when none.
#[test]
fn the_run_totals_round_trip_without_member_text() {
    let secret = "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let totals = RunTotals::new(
        TaskStates {
            total: 2,
            submitted: 1,
            ready: 1,
            ..TaskStates::default()
        },
        MessageTotals {
            sent: 3,
            acked: 1,
            withdrawn: 1,
        },
        vec![usage("worker", 2), usage(secret, 1)],
        Some(1.0),
        2.0,
    );
    let line = serde_json::to_value(&totals).unwrap();
    assert_eq!(line["tasks"]["submitted"], 1);
    assert_eq!(line["messages"]["sent"], 3);
    assert_eq!(line["usage"][0]["input_tokens"], 14);
    assert_eq!(line["wall_time_us"], 1_000_000);
    assert_eq!(line["unlisted_usage"], serde_json::Value::Null);
    assert!(!line.to_string().contains("sk-ant"), "{line}");
    let read: RunTotals = serde_json::from_value(line).unwrap();
    assert_eq!(read, totals);
}
