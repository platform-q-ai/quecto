//! The refusal order of an explicit action (#2011 review R2-H3): the target
//! exists → it names the version it was shown → that version is current → the
//! decision kind offers the action → its executor is composed. Every step is
//! effect-free, so the executors of #2012–#2014 inherit the whole order.
use super::*;
use std::sync::atomic::Ordering;

fn act(target: &str, action: ResumeAction, version: Option<HomeVersion>) -> ResumeRequest {
    ResumeRequest {
        target: target.into(),
        intent: ResumeIntent::Act(action),
        expected_home_version: version,
    }
}

fn current(scope: &SessionHomeScope) -> HomeVersion {
    HomeVersion::of(&saved_identity(), scope)
}

/// A missing session is `not_found` whatever action and token come with it —
/// another session's valid token and no token included.
#[tokio::test]
async fn an_action_on_a_missing_session_is_not_found_before_its_token_is_looked_at() {
    for version in [Some(shown()), None] {
        let case = case(SessionHomeScope::Scoped(folder(ELSEWHERE)));
        let refused = case
            .request(&act("absent", ResumeAction::Locate, version))
            .await
            .expect_err("not found");
        assert!(
            matches!(&refused, ResumeSavedSessionError::NotFound(name) if name == "absent"),
            "{refused:?}"
        );
        case.assert_no_effect();
    }
}

/// The loop's own key gets no exception for an action: an action saves
/// nothing, so a record that is absent stays absent.
#[tokio::test]
async fn an_action_on_the_loops_own_absent_key_is_not_found() {
    let scope = SessionHomeScope::LegacyUnscoped;
    let case = case_on("cli:unsaved", vec![scope], false, false);
    let refused = case
        .request(&act("unsaved", ResumeAction::Associate, Some(shown())))
        .await
        .expect_err("not found");
    assert!(matches!(refused, ResumeSavedSessionError::NotFound(_)));
    case.assert_no_effect();
}

/// An unreadable store answers no action: there is no claimed path to defer to.
#[tokio::test]
async fn an_action_against_an_unreadable_store_is_a_load_failure() {
    let scope = SessionHomeScope::Scoped(folder(ELSEWHERE));
    let case = case(scope.clone());
    case.rig.store.fail_load.store(true, Ordering::SeqCst);
    let refused = case
        .request(&act(
            "saved",
            ResumeAction::OpenOriginal,
            Some(current(&scope)),
        ))
        .await
        .expect_err("load failed");
    assert!(
        matches!(refused, ResumeSavedSessionError::Load(_)),
        "{refused:?}"
    );
    case.assert_no_effect();
}

/// Another session's token, or a token of an earlier home, is stale — before
/// the action is judged offered or available.
#[tokio::test]
async fn an_action_with_a_stale_token_is_stale_before_the_action_is_judged() {
    for action in [ResumeAction::OpenOriginal, ResumeAction::Associate] {
        let case = case(SessionHomeScope::Scoped(folder(ELSEWHERE)));
        let refused = case
            .request(&act("saved", action, Some(shown())))
            .await
            .expect_err("stale");
        assert!(
            matches!(refused, ResumeSavedSessionError::StaleHomeVersion),
            "{action:?}: {refused:?}"
        );
        case.assert_no_effect();
    }
}

/// An action this kind of decision never offers — or any action on a session
/// that simply restores here — is `action_not_offered`, a code of its own.
#[tokio::test]
async fn an_action_the_kind_does_not_offer_is_not_offered() {
    for (scope, action) in [
        (
            SessionHomeScope::Scoped(folder(ELSEWHERE)),
            ResumeAction::Associate,
        ),
        (
            SessionHomeScope::Scoped(folder(ELSEWHERE)),
            ResumeAction::Locate,
        ),
        (SessionHomeScope::LegacyUnscoped, ResumeAction::ForkCurrent),
        (
            SessionHomeScope::Scoped(folder(HERE)),
            ResumeAction::ForkCurrent,
        ),
    ] {
        let case = case(scope.clone());
        let refused = case
            .request(&act("saved", action, Some(current(&scope))))
            .await
            .expect_err("not offered");
        assert!(
            matches!(&refused, ResumeSavedSessionError::ActionNotOffered(a) if *a == action),
            "{scope:?} {action:?}: {refused:?}"
        );
        assert_eq!(refused.code(), "action_not_offered");
        assert!(refused.to_string().contains(action.name()), "{refused}");
        case.assert_no_effect();
    }
}

/// Not offered wins over a composed executor: composition cannot widen a kind.
#[tokio::test]
async fn a_composed_executor_does_not_make_an_unoffered_action_available() {
    let scope = SessionHomeScope::LegacyUnscoped;
    let Case { rig, .. } = case(scope.clone());
    let FreshRig { resume, .. } = rig;
    let resume = resume.with_capabilities(
        ResumeActionCapabilities::cancel_only().with(ResumeAction::OpenOriginal),
    );
    let mut messages = vec![Message::user("live")];
    let rig = build_fresh_rig(FreshOptions::default());
    let mut runtime = rig.runtime();
    let request = act("saved", ResumeAction::OpenOriginal, Some(current(&scope)));
    let refused = resume
        .execute(&request, &mut messages, None, &mut runtime)
        .await
        .expect_err("not offered");
    assert!(matches!(
        refused,
        ResumeSavedSessionError::ActionNotOffered(ResumeAction::OpenOriginal)
    ));
}

/// An undiscoverable current directory refuses an action as it does a restore.
#[tokio::test]
async fn an_action_with_an_undiscoverable_current_directory_is_that_refusal() {
    let scope = SessionHomeScope::Scoped(folder(ELSEWHERE));
    let case = case_with(vec![scope.clone()], false, true);
    let refused = case
        .request(&act(
            "saved",
            ResumeAction::OpenOriginal,
            Some(current(&scope)),
        ))
        .await
        .expect_err("scope unavailable");
    assert!(
        matches!(refused, ResumeSavedSessionError::CurrentScopeUnavailable(_)),
        "{refused:?}"
    );
    case.assert_no_effect();
}
