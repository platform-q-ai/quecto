//! #1721: automatic-turn suspensions are dated by swarm control generation so
//! a later resume can lift a provider-failure suspension, while wakes alone
//! and store rejections never do.
use super::{AgentSession, SuspensionCause};

fn session() -> AgentSession {
    AgentSession::new("m".into(), "k".into())
}

#[test]
fn a_resume_after_the_failure_re_arms_a_provider_suspension() {
    let mut s = session();
    s.suspend_automatic_turns(SuspensionCause::ProviderFailure, Some(4));
    assert!(!s.automatic_turns_allowed);
    assert!(
        !s.resume_after_control_change(4),
        "same generation: no resume happened"
    );
    assert!(!s.automatic_turns_allowed);
    assert!(
        s.resume_after_control_change(6),
        "pause+resume bumped the generation"
    );
    assert!(s.automatic_turns_allowed);
    assert!(!s.resume_after_control_change(9), "nothing left to re-arm");
}

#[test]
fn an_undated_suspension_takes_the_first_wake_as_its_baseline() {
    let mut s = session();
    s.suspend_automatic_turns(SuspensionCause::ProviderFailure, None);
    assert!(
        !s.resume_after_control_change(3),
        "a wake alone must not clear a suspension"
    );
    assert!(!s.automatic_turns_allowed);
    assert!(s.resume_after_control_change(4));
    assert!(s.automatic_turns_allowed);
    // A session that has seen a generation dates its suspension from it.
    let mut t = session();
    t.observe_control_generation(Some(2));
    t.observe_control_generation(None);
    t.observe_control_generation(Some(1));
    t.suspend_automatic_turns(SuspensionCause::ProviderFailure, None);
    assert!(
        !t.resume_after_control_change(2),
        "same generation as the failure"
    );
    assert!(
        t.resume_after_control_change(3),
        "dated at 2, a later generation resumes"
    );
}

#[test]
fn store_rejections_and_explicit_prompts_behave_as_before() {
    let mut s = session();
    s.suspend_automatic_turns(SuspensionCause::StoreRejection, Some(1));
    assert!(
        !s.resume_after_control_change(100),
        "a store rejection waits for a human"
    );
    assert!(!s.automatic_turns_allowed);
    s.resume_automatic_turns();
    assert!(s.automatic_turns_allowed);
    assert!(!s.resume_after_control_change(200), "nothing suspended");
    let mut u = session();
    u.observe_control_generation(Some(1));
    assert!(
        u.automatic_turns_allowed,
        "observing without a suspension is a no-op"
    );
}
