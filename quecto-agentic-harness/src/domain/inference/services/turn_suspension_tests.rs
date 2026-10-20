//! #1721: automatic-turn suspensions are dated by swarm control generation so
//! a later resume can lift a provider-failure suspension, while wakes alone
//! and store rejections never do.
use super::*;

fn session() -> AutomaticTurns {
    AutomaticTurns::default()
}

#[test]
fn a_resume_after_the_failure_re_arms_a_provider_suspension() {
    let mut s = session();
    s.suspend(SuspensionCause::ProviderFailure, Some(4));
    assert!(!s.allowed());
    assert!(
        !s.resume_after_control_change(4),
        "same generation: no resume happened"
    );
    assert!(!s.allowed());
    assert!(
        s.resume_after_control_change(6),
        "pause+resume bumped the generation"
    );
    assert!(s.allowed());
    assert!(!s.resume_after_control_change(9), "nothing left to re-arm");
}

#[test]
fn an_undated_suspension_takes_the_first_wake_as_its_baseline() {
    let mut s = session();
    s.suspend(SuspensionCause::ProviderFailure, None);
    assert!(
        !s.resume_after_control_change(3),
        "a wake alone must not clear a suspension"
    );
    assert!(!s.allowed());
    assert!(s.resume_after_control_change(4));
    assert!(s.allowed());
    // A session that has seen a generation dates its suspension from it, so
    // the first later generation resumes (an undated one would only take
    // it as the baseline).
    let mut t = session();
    t.observe_control_generation(Some(2));
    t.observe_control_generation(None);
    t.observe_control_generation(Some(1));
    t.suspend(SuspensionCause::ProviderFailure, None);
    assert!(
        t.resume_after_control_change(3),
        "dated at 2, a later generation resumes"
    );
    let mut u = session();
    u.observe_control_generation(Some(2));
    u.suspend(SuspensionCause::ProviderFailure, None);
    assert!(
        !u.resume_after_control_change(2),
        "same generation as the failure"
    );
}

#[test]
fn the_owed_resume_turn_does_not_outlive_a_prompt_or_a_later_suspension() {
    let mut s = session();
    s.suspend(SuspensionCause::ProviderFailure, Some(1));
    assert!(!s.needs_provider_dating(), "dated at suspension time");
    assert!(s.resume_after_control_change(2));
    // An explicit prompt does the work the resume turn was owed for.
    s.start(Work::Instruction(Instruction::Prompt));
    assert!(
        !s.take_pending_resume_turn(),
        "prompt consumed the owed turn"
    );
    s.suspend(SuspensionCause::ProviderFailure, Some(2));
    assert!(s.resume_after_control_change(3));
    s.suspend(SuspensionCause::StoreRejection, Some(3));
    assert!(!s.needs_provider_dating());
    assert!(
        !s.take_pending_resume_turn(),
        "a later suspension cancels the owed turn"
    );
    assert!(!session().needs_provider_dating());
    let mut undated = session();
    undated.suspend(SuspensionCause::ProviderFailure, None);
    assert!(
        undated.needs_provider_dating(),
        "a pre-turn fallback needs dating"
    );
}

#[test]
fn store_rejections_and_explicit_prompts_behave_as_before() {
    let mut s = session();
    s.suspend(SuspensionCause::StoreRejection, Some(1));
    assert!(
        !s.resume_after_control_change(100),
        "a store rejection waits for a human"
    );
    assert!(!s.allowed());
    s.start(Work::Instruction(Instruction::Prompt));
    assert!(s.allowed());
    assert!(!s.resume_after_control_change(200), "nothing suspended");
    let mut u = session();
    u.observe_control_generation(Some(1));
    assert!(u.allowed(), "observing without a suspension is a no-op");
}

#[test]
fn every_explicit_instruction_re_arms_any_suspension() {
    for instruction in [
        Instruction::Prompt,
        Instruction::Steer,
        Instruction::FollowUp,
    ] {
        for cause in [
            SuspensionCause::ProviderFailure,
            SuspensionCause::StoreRejection,
        ] {
            let mut turns = session();
            turns.suspend(cause, Some(1));
            assert_eq!(turns.suspension().map(|s| s.cause), Some(cause));
            assert!(turns.admits(Work::Instruction(instruction)));
            assert!(!turns.admits(Work::Automatic), "automatic work waits");
            turns.start(Work::Instruction(instruction));
            assert!(turns.allowed(), "{instruction:?} re-arms {cause:?}");
            assert_eq!(turns.suspension(), None);
        }
    }
}

/// #2618 review: only an instruction is admitted while suspended; a
/// running session admits automatic work, and a follow-up there leaves the
/// turn a resume owed to the run, while a prompt does that work itself.
#[test]
fn the_domain_decides_which_work_starts_a_turn() {
    let mut turns = session();
    assert!(turns.admits(Work::Automatic));
    turns.start(Work::Automatic);
    turns.suspend(SuspensionCause::ProviderFailure, Some(1));
    assert!(turns.resume_after_control_change(2));
    turns.start(Work::Instruction(Instruction::FollowUp));
    assert!(
        turns.take_pending_resume_turn(),
        "a follow-up keeps the owed turn"
    );
    turns.suspend(SuspensionCause::ProviderFailure, Some(2));
    assert!(turns.resume_after_control_change(3));
    turns.start(Work::Instruction(Instruction::Prompt));
    assert!(
        !turns.take_pending_resume_turn(),
        "a prompt does the owed work"
    );
}

#[test]
#[should_panic(expected = "automatic work starts only once admitted")]
fn automatic_work_never_starts_a_suspended_session() {
    let mut turns = session();
    turns.suspend(SuspensionCause::ProviderFailure, Some(1));
    turns.start(Work::Automatic);
}
