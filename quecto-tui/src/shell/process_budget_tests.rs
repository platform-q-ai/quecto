//! The leader-exit budget is derived from the harness's documented teardown
//! numbers: at least the fleet's per-child worst case, never above the
//! harness's own force-exit. The mirrored constants are pinned against the
//! harness sources so a change on either side fails here.

use super::*;

#[test]
fn budget_is_at_least_the_fleet_per_child_worst_case() {
    assert!(LEADER_EXIT_BUDGET >= HARNESS_COMPENSATION_WAIT);
    assert_eq!(HARNESS_OWNED_HANDLE_LADDER, Duration::from_secs(19));
    assert_eq!(HARNESS_COMPENSATION_WAIT, Duration::from_secs(25));
    assert_eq!(LEADER_EXIT_BUDGET, Duration::from_secs(30));
}

#[test]
fn budget_never_exceeds_the_harness_force_exit() {
    assert!(LEADER_EXIT_BUDGET < HARNESS_FORCE_EXIT_AFTER);
    assert_eq!(HARNESS_FORCE_EXIT_AFTER, Duration::from_secs(45));
}

#[test]
fn settling_notice_is_well_inside_the_budget() {
    assert_eq!(SETTLING_NOTICE_AFTER, Duration::from_secs(1));
    assert!(SETTLING_NOTICE_AFTER < LEADER_EXIT_BUDGET);
}

fn harness_source(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../quecto-agentic-harness/src")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
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
    let shutdown = harness_source("interface/cli/uds_shutdown.rs");
    assert_eq!(
        Duration::from_secs(secs_after(&shutdown, "pub const FORCE_EXIT_AFTER")),
        HARNESS_FORCE_EXIT_AFTER
    );
}
