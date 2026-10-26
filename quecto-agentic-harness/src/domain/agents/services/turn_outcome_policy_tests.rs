//! #2708: the parent turn-outcome policy, one assertion per alternative.
use super::*;
use crate::domain::agents::value_objects::turn_declaration::TurnIdentity;
use crate::domain::agents::value_objects::turn_outcome::{
    LIVE_REASON, STALLED_REASON, UNKNOWN_REASON,
};

fn declared(intent: TurnIntent, reason: &str) -> TurnDeclaration {
    TurnDeclaration {
        identity: TurnIdentity {
            session_key: "cli:root".into(),
            outcome_epoch: "epoch".into(),
            turn_id: "turn".into(),
        },
        intent,
        reason: reason.into(),
    }
}

fn settle(outcome: TurnOutcome, reason: &str) -> TurnVerdict {
    TurnVerdict::Settle(outcome, reason.into())
}

const LIVENESS: [WorkLiveness; 3] = [
    WorkLiveness::Live,
    WorkLiveness::Empty,
    WorkLiveness::Unknown,
];

#[test]
fn issue_2708_parent_runtime_empty_waiting_or_undeclared_is_corrected_once_then_stalls() {
    let waiting = declared(TurnIntent::Waiting, "Now working on #2650");
    for declaration in [None, Some(&waiting)] {
        assert_eq!(
            settle_turn(declaration, WorkLiveness::Empty, false),
            TurnVerdict::Correct
        );
        assert_eq!(
            settle_turn(declaration, WorkLiveness::Empty, true),
            settle(TurnOutcome::Stalled, STALLED_REASON)
        );
    }
}

#[test]
fn issue_2708_parent_runtime_live_work_waits_without_correction() {
    let waiting = declared(TurnIntent::Waiting, "Reviewer swarm is running");
    for spent in [false, true] {
        assert_eq!(
            settle_turn(Some(&waiting), WorkLiveness::Live, spent),
            settle(TurnOutcome::Waiting, "Reviewer swarm is running")
        );
        assert_eq!(
            settle_turn(None, WorkLiveness::Live, spent),
            settle(TurnOutcome::Waiting, LIVE_REASON)
        );
    }
}

#[test]
fn issue_2708_parent_runtime_unknown_without_live_needs_the_user_never_a_correction() {
    let waiting = declared(TurnIntent::Waiting, "Running");
    for declaration in [None, Some(&waiting)] {
        for spent in [false, true] {
            assert_eq!(
                settle_turn(declaration, WorkLiveness::Unknown, spent),
                settle(TurnOutcome::NeedsUser, UNKNOWN_REASON)
            );
        }
    }
}

#[test]
fn issue_2708_parent_runtime_blocked_and_asking_need_the_user_whatever_runs() {
    for intent in [TurnIntent::Blocked, TurnIntent::Asking] {
        let declaration = declared(intent, "Which repository?");
        for liveness in LIVENESS {
            for spent in [false, true] {
                assert_eq!(
                    settle_turn(Some(&declaration), liveness, spent),
                    settle(TurnOutcome::NeedsUser, "Which repository?")
                );
            }
        }
    }
}

#[test]
fn issue_2708_parent_runtime_done_is_done_whatever_runs() {
    let declaration = declared(TurnIntent::Done, "Merged #2650");
    for liveness in LIVENESS {
        for spent in [false, true] {
            assert_eq!(
                settle_turn(Some(&declaration), liveness, spent),
                settle(TurnOutcome::Done, "Merged #2650")
            );
        }
    }
}

#[test]
fn issue_2708_parent_runtime_terminal_outcomes_are_exactly_needs_user_stalled_done() {
    assert!(!TurnOutcome::Waiting.is_terminal());
    for outcome in [
        TurnOutcome::NeedsUser,
        TurnOutcome::Stalled,
        TurnOutcome::Done,
    ] {
        assert!(outcome.is_terminal(), "{outcome:?}");
    }
}

#[test]
fn issue_2708_parent_runtime_live_beats_unknown_and_unknown_is_never_empty() {
    use WorkLiveness::{Empty, Live, Unknown};
    assert_eq!(Live.and(Unknown), Live);
    assert_eq!(Unknown.and(Live), Live);
    assert_eq!(Empty.and(Unknown), Unknown);
    assert_eq!(Unknown.and(Empty), Unknown);
    assert_eq!(Empty.and(Empty), Empty);
    assert_eq!(Live.and(Empty), Live);
}

const ROOT: LaunchFacts = LaunchFacts {
    spawned: false,
    coordinator: false,
    has_parent_id: false,
    has_parent_control: false,
    swarm_member: false,
};

#[test]
fn issue_2708_cli_parent_only_the_exact_root_launch_is_trusted() {
    assert!(is_trusted_root(ROOT));
    let excluded = [
        (
            "child",
            LaunchFacts {
                spawned: true,
                has_parent_id: true,
                ..ROOT
            },
        ),
        (
            "worker",
            LaunchFacts {
                spawned: true,
                swarm_member: true,
                ..ROOT
            },
        ),
        (
            "child_coordinator",
            LaunchFacts {
                spawned: true,
                coordinator: true,
                has_parent_id: true,
                ..ROOT
            },
        ),
        (
            "independent_coordinator",
            LaunchFacts {
                coordinator: true,
                ..ROOT
            },
        ),
        (
            "bare_participant",
            LaunchFacts {
                swarm_member: true,
                ..ROOT
            },
        ),
        (
            "parent_controlled",
            LaunchFacts {
                has_parent_control: true,
                ..ROOT
            },
        ),
        (
            "parent_id_only",
            LaunchFacts {
                has_parent_id: true,
                ..ROOT
            },
        ),
    ];
    for (name, facts) in excluded {
        assert!(!is_trusted_root(facts), "{name} must not be a trusted root");
    }
}

#[test]
fn issue_2708_cli_parent_the_switch_is_on_only_for_exactly_one() {
    assert!(turn_outcomes_switched_on(Some("1")));
    for off in [
        None,
        Some(""),
        Some("0"),
        Some("true"),
        Some("yes"),
        Some(" 1"),
        Some("1 "),
    ] {
        assert!(!turn_outcomes_switched_on(off), "{off:?} is not the switch");
    }
}

#[test]
fn issue_2708_protocol_episode_ids_are_one_to_512_bytes_untrimmed() {
    assert!(admissible_episode_id("e"));
    assert!(admissible_episode_id(" \t"));
    assert!(admissible_episode_id(&"a".repeat(512)));
    assert!(!admissible_episode_id(&"a".repeat(513)));
    assert!(!admissible_episode_id(""));
    // 170 three-byte scalars are 510 bytes; one more crosses the bound.
    assert!(admissible_episode_id(&format!("{}ab", "界".repeat(170))));
    assert!(!admissible_episode_id(&"界".repeat(171)));
}
