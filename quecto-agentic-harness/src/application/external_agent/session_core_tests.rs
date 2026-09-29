//! The session core's own contracts (#2287), for the calls the session
//! use case makes only once they already hold: a turn is lost only once
//! its skipped line's grace has run out, and only the running turn is
//! interrupted.

use super::*;
use crate::domain::external_agent::stream::{SkippedLine, SkippedLineReason};

/// A core running turn 1, whose last event was a skipped line whose grace
/// runs out at `grace_end`.
fn skipped_in_turn_one(grace_end: u64) -> SessionCore {
    let mut core = SessionCore {
        phase: SessionPhase::Idle,
        ..SessionCore::default()
    };
    assert_eq!(
        core.admit("one", None),
        Ok(Admission::Write(PromptAccepted::Started { turn: 1 }))
    );
    core.fold(
        &ExternalAgentEvent::LineSkipped(SkippedLine {
            reason: SkippedLineReason::OverCap,
            bytes: 9,
        }),
        AgentClockInstant(0),
        AgentClockInstant(grace_end),
    );
    assert_eq!(core.wait(), Wait::Grace(AgentClockInstant(grace_end)));
    core
}

#[test]
fn a_turn_is_lost_only_once_its_grace_has_run_out() {
    let core = skipped_in_turn_one(100);
    assert_eq!(core.lost_turn(AgentClockInstant(0)), None);
    assert_eq!(core.lost_turn(AgentClockInstant(99)), None);
    assert_eq!(core.lost_turn(AgentClockInstant(100)), Some(1));
    assert_eq!(core.lost_turn(AgentClockInstant(101)), Some(1));
}

#[test]
fn an_interrupted_turn_is_overdue_only_once_its_deadline_has_passed() {
    let mut core = skipped_in_turn_one(100);
    assert!(core.interrupting(1, AgentClockInstant(500)));
    assert_eq!(core.phase, SessionPhase::Interrupting { turn: 1 });
    assert_eq!(core.wait(), Wait::Until(AgentClockInstant(500)));
    assert_eq!(core.lost_turn(AgentClockInstant(600)), None, "not busy");
    assert_eq!(core.interrupt_overdue(AgentClockInstant(499)), None);
    assert_eq!(core.interrupt_overdue(AgentClockInstant(500)), Some(1));
    assert!(!core.ended());
    core.end();
    assert!(core.ended());
    assert_eq!(core.interrupt_overdue(AgentClockInstant(600)), None);
}

#[test]
#[should_panic(expected = "only the running turn 2 is interrupted")]
fn only_the_running_turn_is_interrupted() {
    let mut core = skipped_in_turn_one(100);
    core.interrupting(2, AgentClockInstant(500));
}

#[test]
fn an_ended_member_is_not_interrupted() {
    let mut core = skipped_in_turn_one(100);
    assert_eq!(core.end(), (Some(1), 0));
    assert!(!core.interrupting(1, AgentClockInstant(500)));
    assert_eq!(core.phase, SessionPhase::Ended);
}
