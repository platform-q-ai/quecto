//! The leader-exit budget is derived from the harness's documented teardown
//! numbers: at least the fleet's per-child worst case, never above the
//! harness's own force-exit. The mirrored constants are pinned against the
//! harness sources so a change on either side fails here.

use super::*;

#[test]
fn settle_budget_covers_the_fleet_batches_for_the_roster() {
    assert_eq!(HARNESS_OWNED_HANDLE_LADDER, Duration::from_secs(19));
    assert_eq!(HARNESS_COMPENSATION_WAIT, Duration::from_secs(25));
    // One batch covers up to 8 children; 9 need two; 17+ need the 3-pass cap.
    assert_eq!(fleet_batches(Some(0)), 1);
    assert_eq!(fleet_batches(Some(8)), 1);
    assert_eq!(fleet_batches(Some(9)), 2);
    assert_eq!(fleet_batches(Some(16)), 2);
    assert_eq!(fleet_batches(Some(17)), 3);
    assert_eq!(fleet_batches(Some(1_000)), 3);
    assert_eq!(fleet_batches(None), 3);
    assert_eq!(fleet_settle_budget(Some(0)), Duration::from_secs(30));
    assert_eq!(fleet_settle_budget(Some(9)), Duration::from_secs(55));
    assert_eq!(fleet_settle_budget(None), Duration::from_secs(80));
    assert_eq!(LeaderBudget::for_children(None), LeaderBudget::WORST_CASE);
    for children in [Some(0), Some(3), Some(9), Some(40), None] {
        assert!(
            LeaderBudget::for_children(children).settle
                >= HARNESS_COMPENSATION_WAIT.saturating_mul(fleet_batches(children)),
            "settle wait always covers the batches the fleet needs"
        );
    }
}

#[test]
fn force_wait_is_the_harness_force_exit_after_a_repeated_signal() {
    assert_eq!(HARNESS_FORCE_EXIT_AFTER, Duration::from_secs(45));
    assert_eq!(LeaderBudget::WORST_CASE.force, HARNESS_FORCE_EXIT_AFTER);
    assert_eq!(
        LeaderBudget::for_children(Some(2)).force,
        HARNESS_FORCE_EXIT_AFTER
    );
    assert_eq!(
        LeaderBudget::WORST_CASE.total(),
        Duration::from_secs(80 + 45 + 2)
    );
}

#[test]
fn settling_notice_is_well_inside_the_budget() {
    assert_eq!(SETTLING_NOTICE_AFTER, Duration::from_secs(1));
    assert!(SETTLING_NOTICE_AFTER < LeaderBudget::for_children(Some(0)).settle);
}

fn harness_source(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../quecto-agentic-harness/src")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn usize_after(source: &str, anchor: &str) -> usize {
    let at = source
        .find(anchor)
        .unwrap_or_else(|| panic!("anchor {anchor:?} not found"));
    let rest = &source[at + anchor.len()..];
    let start = rest.find('=').expect("assignment") + 1;
    let end = rest[start..].find(';').expect("semicolon") + start;
    rest[start..end].trim().parse().expect("integer")
}

fn secs_after(source: &str, anchor: &str) -> u64 {
    let at = source
        .find(anchor)
        .unwrap_or_else(|| panic!("anchor {anchor:?} not found"));
    let rest = &source[at + anchor.len()..];
    let start = rest.find("from_secs(").expect("from_secs after anchor") + "from_secs(".len();
    let end = rest[start..].find(')').expect("closing paren") + start;
    rest[start..end].trim().parse().expect("integer seconds")
}

/// The mirrored numbers match the harness's own constants.
#[test]
fn mirrored_constants_match_the_harness_sources() {
    let routing = harness_source("infrastructure/processes/direct_child_routing.rs");
    assert_eq!(
        Duration::from_secs(secs_after(&routing, "pub const PROTOCOL_ACK_TIMEOUT")),
        HARNESS_PROTOCOL_ACK_TIMEOUT
    );
    let supervisor = harness_source("infrastructure/processes/owned_child_supervisor.rs");
    let default_block = &supervisor[supervisor.find("pub const DEFAULT: Self").unwrap()..];
    assert_eq!(
        Duration::from_secs(secs_after(default_block, "exit_after_ack:")),
        HARNESS_EXIT_AFTER_ACK
    );
    assert_eq!(
        Duration::from_secs(secs_after(default_block, "term_grace:")),
        HARNESS_TERM_GRACE
    );
    assert_eq!(
        Duration::from_secs(secs_after(default_block, "kill_grace:")),
        HARNESS_KILL_GRACE
    );
    let registry = harness_source("infrastructure/tools/subagent_teardown_registry.rs");
    assert_eq!(
        Duration::from_secs(secs_after(&registry, "const COMPENSATION_WAIT_SLACK")),
        HARNESS_COMPENSATION_WAIT_SLACK
    );
    let fleet = harness_source("application/subagents/use_cases/terminate_all_delegated_agents.rs");
    assert_eq!(
        usize_after(&fleet, "pub const DEFAULT_SETTLEMENT_BOUND: usize"),
        HARNESS_SETTLEMENT_BOUND
    );
    assert_eq!(
        usize_after(&fleet, "const MAX_PASSES: usize"),
        HARNESS_MAX_PASSES as usize
    );
    let shutdown = harness_source("interface/cli/uds_shutdown.rs");
    assert_eq!(
        Duration::from_secs(secs_after(&shutdown, "pub const FORCE_EXIT_AFTER")),
        HARNESS_FORCE_EXIT_AFTER
    );
}
