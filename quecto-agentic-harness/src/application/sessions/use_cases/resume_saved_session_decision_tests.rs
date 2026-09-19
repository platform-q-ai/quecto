//! The typed resume decisions of the transaction (#2011): what is decided
//! without any effect, what is re-decided under the claim, and that no
//! refusal replaces the session or leaks a claim.
use crate::application::sessions::dto::{
    ActionAvailability, ResumeActionCapabilities, ResumeIntent, ResumeOutcome, ResumeRequest,
    ResumeSavedSessionError,
};
use crate::application::sessions::ports::session_home::{
    HomeCatalogueSnapshot, SessionHomeCatalogue, WorkspaceDiscovery,
};
use crate::application::sessions::session_home::SessionHomeContext;
use crate::application::sessions::use_cases::start_fresh_rig::{
    FreshOptions, FreshRig, OLD_KEY, build_fresh_rig,
};
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::resume_decision::{HomeVersion, ResumeAction, ResumeDecisionKind};
use crate::domain::session::Session;
use crate::domain::session_home::{
    AssociationProvenance, SessionHome, SessionHomeScope, WorkspaceGroup,
};
use crate::domain::session_identity::SessionIdentity;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const HERE: &str = "/work/here";
const ELSEWHERE: &str = "/work/elsewhere";
const GONE: &str = "/work/gone";

fn saved_identity() -> SessionIdentity {
    SessionIdentity::from_persisted_key("cli:saved")
}

/// Some version a client was shown; an action's admission needs one.
fn shown() -> HomeVersion {
    HomeVersion::of(&saved_identity(), &SessionHomeScope::LegacyUnscoped)
}

fn folder(dir: &str) -> SessionHome {
    SessionHome {
        execution_dir: PathBuf::from(dir),
        group: WorkspaceGroup::Folder {
            directory: PathBuf::from(dir),
        },
        provenance: AssociationProvenance::SavedHere,
    }
}

/// Scripted authority and facts: `scopes` are handed out read by read (the
/// last one repeats), so a home can change between the pre-flight and the
/// claimed re-check; `GONE` cannot be discovered; `here_is_git` turns the
/// current directory into a repository.
struct Facts {
    scopes: Mutex<Vec<SessionHomeScope>>,
    reads: Mutex<usize>,
    here_is_git: bool,
    current_fails: bool,
}

impl SessionHomeCatalogue for Facts {
    fn metadata(
        &self,
    ) -> crate::application::sessions::ports::session_home::Answer<
        '_,
        crate::application::sessions::ports::session_home::SessionMetadataSnapshot,
    > {
        panic!("only the metadata search asks the metadata query")
    }
    fn read(&self, _: &SessionIdentity) -> Result<SessionHomeScope, DomainError> {
        *self.reads.lock().unwrap() += 1;
        let mut scopes = self.scopes.lock().unwrap();
        Ok(if scopes.len() > 1 {
            scopes.remove(0)
        } else {
            scopes[0].clone()
        })
    }
    fn list(&self) -> Result<HomeCatalogueSnapshot, DomainError> {
        Err(DomainError::Session("the derived index is corrupt".into()))
    }
    fn record_new(&self, _: &SessionIdentity, _: &SessionHome) -> Result<(), DomainError> {
        Ok(())
    }
    fn discard_orphan(&self, _: &SessionIdentity) -> Result<(), DomainError> {
        Ok(())
    }
}

impl WorkspaceDiscovery for Facts {
    fn discover(&self, path: &Path) -> Result<SessionHome, DomainError> {
        if path == Path::new(GONE) || (self.current_fails && path == Path::new(HERE)) {
            return Err(DomainError::Session("No such file or directory".into()));
        }
        let mut home = folder(&path.to_string_lossy());
        if self.here_is_git && path == Path::new(HERE) {
            home.group = WorkspaceGroup::Git {
                common_dir: PathBuf::from("/work/here/.git"),
            };
        }
        Ok(home)
    }
}

struct Case {
    rig: FreshRig,
    facts: Arc<Facts>,
    current: &'static str,
}

fn case_with(scopes: Vec<SessionHomeScope>, here_is_git: bool, current_fails: bool) -> Case {
    case_on(OLD_KEY, scopes, here_is_git, current_fails)
}

/// A case whose loop stands for `current` (its own key when `cli:saved`).
fn case_on(
    current: &'static str,
    scopes: Vec<SessionHomeScope>,
    here_is_git: bool,
    current_fails: bool,
) -> Case {
    let facts = Arc::new(Facts {
        scopes: Mutex::new(scopes),
        reads: Mutex::new(0),
        here_is_git,
        current_fails,
    });
    let home = SessionHomeContext::at(facts.clone(), facts.clone(), PathBuf::from(HERE));
    let rig = build_fresh_rig(FreshOptions {
        home: Some(home),
        current_identity: current,
        ..FreshOptions::default()
    });
    rig.store.seed(Session {
        key: SessionIdentity::from_persisted_key("cli:saved"),
        messages: vec![Message::user("saved-1")],
        workflow_run: None,
        subagent_roster: Vec::new(),
    });
    Case {
        rig,
        facts,
        current,
    }
}

fn case(scope: SessionHomeScope) -> Case {
    case_with(vec![scope], false, false)
}

impl Case {
    async fn request(
        &self,
        request: &ResumeRequest,
    ) -> Result<ResumeOutcome, ResumeSavedSessionError> {
        let mut messages = vec![Message::user("live")];
        let mut runtime = self.rig.runtime();
        let fleet = self.rig.settled_fleet();
        let outcome = self
            .rig
            .resume
            .execute(request, &mut messages, Some(&fleet), &mut runtime)
            .await;
        if outcome.is_err() {
            assert_eq!(messages.len(), 1, "a refusal keeps the live conversation");
            assert_eq!(messages[0].content, "live");
            assert_eq!(
                self.rig.identity(),
                self.current,
                "a refusal keeps the identity"
            );
        }
        outcome
    }

    async fn restore(&self) -> Result<ResumeOutcome, ResumeSavedSessionError> {
        self.request(&ResumeRequest::restore("saved")).await
    }

    /// Nothing was settled, saved, claimed, released or switched — and no
    /// transcript was read: the pre-flight asks the store for existence only.
    fn assert_no_effect(&self) {
        let effects = self.rig.journal();
        assert_eq!(effects, Vec::<String>::new(), "no effect at all");
        assert!(self.rig.store.claimed.lock().unwrap().is_empty());
    }
}

fn decision_of(
    result: Result<ResumeOutcome, ResumeSavedSessionError>,
) -> crate::application::sessions::dto::ResumeDecision {
    match result {
        Err(ResumeSavedSessionError::Decision(decision)) => *decision,
        other => panic!("decision expected: {other:?}"),
    }
}

fn offered(decision: &crate::application::sessions::dto::ResumeDecision) -> Vec<ResumeAction> {
    decision.offers.iter().map(|offer| offer.action).collect()
}

#[tokio::test]
async fn the_same_execution_directory_restores() {
    let case = case(SessionHomeScope::Scoped(folder(HERE)));
    let outcome = case.restore().await.expect("restored");
    let ResumeOutcome::Resumed(resumed) = outcome else {
        panic!("resumed expected");
    };
    assert_eq!(resumed.identity.runtime_key(), "cli:saved");
    assert_eq!(case.rig.identity(), "cli:saved");
    // The pre-flight and the claimed re-check each read the authority.
    assert_eq!(*case.facts.reads.lock().unwrap(), 2);
}

#[tokio::test]
async fn another_folder_is_a_cross_folder_decision_with_no_effect() {
    let saved = SessionHomeScope::Scoped(folder(ELSEWHERE));
    let case = case(saved.clone());
    let decision = decision_of(case.restore().await);
    assert_eq!(decision.kind, ResumeDecisionKind::CrossFolder);
    assert_eq!(
        offered(&decision),
        [
            ResumeAction::OpenOriginal,
            ResumeAction::ForkCurrent,
            ResumeAction::Cancel
        ]
    );
    assert_eq!(decision.execution_dir, Some(PathBuf::from(ELSEWHERE)));
    assert_eq!(
        decision.home_version,
        HomeVersion::of(&saved_identity(), &saved)
    );
    assert_eq!(decision.target.identity.runtime_key(), "cli:saved");
    for offer in &decision.offers {
        let cancel = offer.action == ResumeAction::Cancel;
        assert_eq!(offer.availability == ActionAvailability::Available, cancel);
    }
    assert!(decision.offer(ResumeAction::Locate).is_none());
    case.assert_no_effect();
}

#[tokio::test]
async fn an_unobservable_home_is_a_home_missing_decision_carrying_the_reason() {
    let case = case(SessionHomeScope::Scoped(folder(GONE)));
    let decision = decision_of(case.restore().await);
    assert_eq!(decision.kind, ResumeDecisionKind::HomeMissing);
    assert_eq!(
        offered(&decision),
        [
            ResumeAction::Locate,
            ResumeAction::ForkCurrent,
            ResumeAction::Cancel
        ]
    );
    assert!(decision.detail.as_deref().unwrap().contains("No such file"));
    case.assert_no_effect();
}

#[tokio::test]
async fn a_changed_group_in_the_same_directory_is_a_home_changed_decision() {
    let case = case_with(vec![SessionHomeScope::Scoped(folder(HERE))], true, false);
    let decision = decision_of(case.restore().await);
    assert_eq!(decision.kind, ResumeDecisionKind::HomeChanged);
    case.assert_no_effect();
}

#[tokio::test]
async fn uninterpretable_metadata_is_a_home_unknown_decision_without_a_path() {
    let case = case(SessionHomeScope::Unavailable(
        "unsupported home version".into(),
    ));
    let decision = decision_of(case.restore().await);
    assert_eq!(decision.kind, ResumeDecisionKind::HomeUnknown);
    assert_eq!(decision.execution_dir, None);
    assert_eq!(decision.detail.as_deref(), Some("unsupported home version"));
    case.assert_no_effect();
}

#[tokio::test]
async fn a_legacy_session_is_a_decision_offering_association_and_cancel_only() {
    let case = case(SessionHomeScope::LegacyUnscoped);
    let decision = decision_of(case.restore().await);
    assert_eq!(decision.kind, ResumeDecisionKind::LegacyUnscoped);
    assert_eq!(
        offered(&decision),
        [ResumeAction::Associate, ResumeAction::Cancel]
    );
    let text = decision.to_string();
    assert!(text.contains("explicit first association"), "{text}");
    assert!(text.contains("stays listed under All Folders"), "{text}");
    case.assert_no_effect();
}

#[tokio::test]
async fn an_undiscoverable_current_directory_is_a_refusal_not_a_decision() {
    let case = case_with(vec![SessionHomeScope::Scoped(folder(HERE))], false, true);
    let refused = case.restore().await.expect_err("refused");
    assert!(
        matches!(refused, ResumeSavedSessionError::CurrentScopeUnavailable(_)),
        "{refused:?}"
    );
    assert_eq!(refused.code(), "current_scope_unavailable");
    case.assert_no_effect();
}

#[tokio::test]
async fn a_stale_expected_version_is_refused_before_any_effect() {
    let case = case(SessionHomeScope::Scoped(folder(HERE)));
    let request = ResumeRequest {
        target: "saved".into(),
        intent: ResumeIntent::Restore,
        expected_home_version: Some(HomeVersion::of(
            &saved_identity(),
            &SessionHomeScope::LegacyUnscoped,
        )),
    };
    let refused = case.request(&request).await.expect_err("stale");
    assert!(matches!(refused, ResumeSavedSessionError::StaleHomeVersion));
    case.assert_no_effect();
}

#[tokio::test]
async fn the_current_expected_version_restores() {
    let scope = SessionHomeScope::Scoped(folder(HERE));
    let case = case(scope.clone());
    let request = ResumeRequest {
        target: "saved".into(),
        intent: ResumeIntent::Restore,
        expected_home_version: Some(HomeVersion::of(&saved_identity(), &scope)),
    };
    assert!(matches!(
        case.request(&request).await,
        Ok(ResumeOutcome::Resumed(_))
    ));
}

/// The authority changes between the pre-flight and the claim: the claimed
/// re-check is the one that counts, and its refusal releases the claim.
#[tokio::test]
async fn a_home_that_changes_after_the_preflight_is_refused_under_the_claim() {
    let listed = SessionHomeScope::Scoped(folder(HERE));
    let moved = SessionHomeScope::Scoped(folder(ELSEWHERE));
    let case = case_with(vec![listed.clone(), moved], false, false);
    let request = ResumeRequest {
        target: "saved".into(),
        intent: ResumeIntent::Restore,
        expected_home_version: Some(HomeVersion::of(&saved_identity(), &listed)),
    };
    let refused = case
        .request(&request)
        .await
        .expect_err("stale under the claim");
    assert!(matches!(refused, ResumeSavedSessionError::StaleHomeVersion));
    let journal = case.rig.journal();
    assert!(
        journal.contains(&"store.claim(cli:saved)".to_string()),
        "{journal:?}"
    );
    assert_eq!(
        journal.last().unwrap(),
        "store.release(cli:saved)@active=cli:departing",
        "the target claim is released"
    );
    assert!(
        !journal.iter().any(|entry| entry.starts_with("history.")),
        "{journal:?}"
    );
}

/// #1995 under #2011: the same refusal under the claim of the loop's OWN key
/// releases nothing — the loop keeps owning the session it stands for.
#[tokio::test]
async fn a_claimed_refusal_of_the_loops_own_key_keeps_its_claim() {
    let case = case_on(
        "cli:saved",
        vec![
            SessionHomeScope::Scoped(folder(HERE)),
            SessionHomeScope::Scoped(folder(ELSEWHERE)),
        ],
        false,
        false,
    );
    let decision = decision_of(case.restore().await);
    assert_eq!(decision.kind, ResumeDecisionKind::CrossFolder);
    let journal = case.rig.journal();
    assert!(
        journal.contains(&"store.claim(cli:saved)".to_string()),
        "{journal:?}"
    );
    assert!(
        case.rig.store.released.lock().unwrap().is_empty(),
        "the loop's own claim is never released: {journal:?}"
    );
}

#[tokio::test]
async fn without_an_expected_version_the_claimed_recheck_still_decides() {
    let case = case_with(
        vec![
            SessionHomeScope::Scoped(folder(HERE)),
            SessionHomeScope::Scoped(folder(ELSEWHERE)),
        ],
        false,
        false,
    );
    let decision = decision_of(case.restore().await);
    assert_eq!(decision.kind, ResumeDecisionKind::CrossFolder);
    assert_eq!(
        case.rig.journal().last().unwrap(),
        "store.release(cli:saved)@active=cli:departing"
    );
}

#[tokio::test]
async fn an_exact_miss_is_not_found_and_never_a_neighbouring_key() {
    let case = case(SessionHomeScope::Scoped(folder(HERE)));
    for near in ["save", "saved-", "sav", "cli:save"] {
        let refused = case
            .request(&ResumeRequest::restore(near))
            .await
            .expect_err("not found");
        assert!(
            matches!(&refused, ResumeSavedSessionError::NotFound(name) if name == near),
            "{near}: {refused:?}"
        );
        assert_eq!(refused.code(), "not_found");
    }
    // Known absent before any effect: nothing settled, saved or claimed.
    case.assert_no_effect();
}

/// The exact-key path reads the authority and never the derived index: the
/// index here fails every listing.
#[tokio::test]
async fn a_corrupt_index_cannot_block_exact_key_resolution() {
    let case = case(SessionHomeScope::Scoped(folder(HERE)));
    assert!(case.facts.list().is_err());
    assert!(matches!(
        case.restore().await,
        Ok(ResumeOutcome::Resumed(_))
    ));
}

#[tokio::test]
async fn cancel_has_no_effect_at_all() {
    let case = case(SessionHomeScope::Scoped(folder(ELSEWHERE)));
    let request = ResumeRequest {
        target: "saved".into(),
        intent: ResumeIntent::Act(ResumeAction::Cancel),
        expected_home_version: None,
    };
    let outcome = case.request(&request).await.expect("cancelled");
    assert_eq!(
        outcome,
        ResumeOutcome::Cancelled {
            name: "saved".into()
        }
    );
    assert_eq!(case.rig.identity(), OLD_KEY);
    assert_eq!(*case.facts.reads.lock().unwrap(), 0);
    case.assert_no_effect();
}

#[tokio::test]
async fn every_offered_action_is_refused_unavailable_and_never_substituted() {
    let elsewhere = SessionHomeScope::Scoped(folder(ELSEWHERE));
    let gone = SessionHomeScope::Scoped(folder(GONE));
    for (scope, action) in [
        (&elsewhere, ResumeAction::OpenOriginal),
        (&elsewhere, ResumeAction::ForkCurrent),
        (&gone, ResumeAction::Locate),
        (&SessionHomeScope::LegacyUnscoped, ResumeAction::Associate),
    ] {
        let case = case(scope.clone());
        let request = ResumeRequest {
            target: "saved".into(),
            intent: ResumeIntent::Act(action),
            expected_home_version: Some(HomeVersion::of(&saved_identity(), scope)),
        };
        let refused = case.request(&request).await.expect_err("unavailable");
        assert!(
            matches!(
                &refused,
                ResumeSavedSessionError::ActionUnavailable { action: refused_action, reason }
                    if *refused_action == action && !reason.is_empty()
            ),
            "{action:?}: {refused:?}"
        );
        case.assert_no_effect();
    }
}

#[tokio::test]
async fn a_composed_executor_makes_its_action_available_and_owned_elsewhere() {
    let base = case(SessionHomeScope::Scoped(folder(ELSEWHERE)));
    let Case { rig, facts, .. } = base;
    let FreshRig { resume, .. } = rig;
    let resume = resume.with_capabilities(
        ResumeActionCapabilities::cancel_only().with(ResumeAction::OpenOriginal),
    );
    let mut messages = vec![Message::user("live")];
    let rig = build_fresh_rig(FreshOptions::default());
    let mut runtime = rig.runtime();
    let decision = decision_of(
        resume
            .execute(
                &ResumeRequest::restore("saved"),
                &mut messages,
                None,
                &mut runtime,
            )
            .await,
    );
    let availability = |action| decision.offer(action).unwrap().availability.clone();
    assert_eq!(
        availability(ResumeAction::OpenOriginal),
        ActionAvailability::Available
    );
    assert!(matches!(
        availability(ResumeAction::ForkCurrent),
        ActionAvailability::Unavailable(_)
    ));
    let text = decision.to_string();
    assert!(
        text.contains("open_original /") && text.contains("fork_current (unavailable)"),
        "{text}"
    );
    // The restore owner never executes it: the executor has its own transaction.
    let request = ResumeRequest {
        target: "saved".into(),
        intent: ResumeIntent::Act(ResumeAction::OpenOriginal),
        expected_home_version: Some(decision.home_version.clone()),
    };
    let refused = resume
        .execute(&request, &mut messages, None, &mut runtime)
        .await
        .expect_err("owned elsewhere");
    assert!(matches!(
        refused,
        ResumeSavedSessionError::ActionExecutedElsewhere(ResumeAction::OpenOriginal)
    ));
    assert_eq!(refused.code(), "action_executed_elsewhere");
    assert!(*facts.reads.lock().unwrap() >= 1);
}

#[tokio::test]
async fn an_ephemeral_loop_and_an_invalid_key_refuse_every_intent() {
    let rig = build_fresh_rig(FreshOptions {
        ephemeral: true,
        ..FreshOptions::default()
    });
    let mut messages = Vec::new();
    let mut runtime = rig.runtime();
    for intent in [
        ResumeIntent::Restore,
        ResumeIntent::Act(ResumeAction::Cancel),
    ] {
        let request = ResumeRequest {
            target: "saved".into(),
            intent,
            expected_home_version: None,
        };
        let refused = rig
            .resume
            .execute(&request, &mut messages, None, &mut runtime)
            .await
            .expect_err("ephemeral");
        assert!(matches!(refused, ResumeSavedSessionError::Ephemeral));
    }
    let case = case(SessionHomeScope::Scoped(folder(HERE)));
    let refused = case
        .request(&ResumeRequest::restore("bad name!"))
        .await
        .expect_err("invalid");
    assert!(matches!(refused, ResumeSavedSessionError::InvalidName));
    case.assert_no_effect();
}

#[path = "resume_saved_session_preflight_tests.rs"]
mod preflight;
