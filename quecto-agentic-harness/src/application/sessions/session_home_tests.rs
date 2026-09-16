//! Real-adapter application acceptance: scope facts never authorize cross-folder restore.
#![cfg(test)]
use super::super::use_cases::{DepartingChildren, ListSessions, ResumeSavedSession, SaveSession};
use super::*;
use crate::application::durable_prefix::DurablePrefixLatch;
use crate::application::sessions::{
    active_session::ActiveSessionState,
    dto::{ListSessionsRequest, SessionListQuery, SessionListScope},
    ports::SessionStore,
};
use crate::domain::{message::Message, session::Session};
use crate::infrastructure::{
    persistence::{
        session_home_catalogue::FileSessionHomeCatalogue, session_layout::FlatSessionLayout,
        session_store::FileSessionStore,
    },
    workspace::git_scope_discovery::GitScopeDiscovery,
};

#[tokio::test]
async fn persisted_home_is_local_but_legacy_and_foreign_startup_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let here = temp.path().join("here");
    let elsewhere = temp.path().join("elsewhere");
    std::fs::create_dir(&here).unwrap();
    std::fs::create_dir(&elsewhere).unwrap();
    let store = Arc::new(FileSessionStore::new(FlatSessionLayout::new(temp.path())));
    let context = SessionHomeContext {
        catalogue: Arc::new(FileSessionHomeCatalogue::with_store(
            FlatSessionLayout::new(temp.path()),
            store.clone(),
        )),
        discovery: Arc::new(GitScopeDiscovery::default()),
        execution_dir: here,
    };
    let identity = SessionIdentity::user_chat("chat-home").unwrap();
    let state = Arc::new(tokio::sync::RwLock::new(ActiveSessionState::new(
        identity.clone(),
    )));
    let save = Arc::new(
        SaveSession::new(
            state.clone(),
            store.clone(),
            Arc::new(DurablePrefixLatch::default()),
            None,
            None,
            false,
        )
        .with_home(context.clone()),
    );
    save.save(
        &mut vec![Message::user("persisted here")],
        crate::application::sessions::dto::SaveTrigger::Routine,
    )
    .await
    .unwrap();
    assert!(matches!(
        context.catalogue.read(&identity).unwrap(),
        SessionHomeScope::Scoped(_)
    ));
    let legacy = SessionIdentity::user_chat("chat-legacy").unwrap();
    let mut session = Session::new(legacy.clone());
    session.messages.push(Message::user("old"));
    store.save(&session).await.unwrap();
    let listed = ListSessions::new(store.clone())
        .with_home(context.clone())
        .discover(&ListSessionsRequest {
            query: SessionListQuery::All,
            scope: SessionListScope::Local,
        })
        .await
        .unwrap();
    assert_eq!(listed.sessions.len(), 1);
    assert_eq!(listed.sessions[0].summary.key, identity.runtime_key());
    assert!(listed.sessions[0].resume_eligible);
    let foreign = SessionHomeContext {
        execution_dir: elsewhere,
        ..context.clone()
    };
    let resume = ResumeSavedSession::new(
        state.clone(),
        save.clone(),
        store.clone(),
        Arc::new(DepartingChildren::new(None)),
        false,
    )
    .with_home(foreign);
    assert!(matches!(
        resume.open_at_startup().await,
        Err(crate::application::sessions::dto::ResumeSavedSessionError::Scope(_))
    ));
    assert_eq!(state.read().await.identity(), &identity);
    let legacy_state = Arc::new(tokio::sync::RwLock::new(ActiveSessionState::new(legacy)));
    let resume = ResumeSavedSession::new(
        legacy_state,
        save,
        store,
        Arc::new(DepartingChildren::new(None)),
        false,
    )
    .with_home(context);
    assert!(matches!(
        resume.open_at_startup().await,
        Err(crate::application::sessions::dto::ResumeSavedSessionError::Scope(_))
    ));
}

struct UntouchedRuntime;
impl crate::application::sessions::ports::session_runtime::TurnAccountingReset
    for UntouchedRuntime
{
    fn history_replaced(&mut self, _: usize) {
        panic!("refusal must not replace runtime");
    }
}
impl crate::application::sessions::ports::SessionKeyPropagation for UntouchedRuntime {
    fn session_key_changed(&mut self, _: &SessionIdentity) {
        panic!("refusal must not replace identity");
    }
}
impl crate::application::sessions::ports::SessionSwitchRuntime for UntouchedRuntime {
    fn reset_effort_to_default(&mut self) {
        panic!("refusal must preserve effort");
    }
    fn reset_workflow(&mut self) {
        panic!("refusal must preserve workflow");
    }
    fn restore_workflow(&mut self, _: crate::domain::workflow::WorkflowRunPersisted) {
        panic!("refusal must preserve workflow");
    }
}

#[tokio::test]
async fn exact_corrupt_home_refusal_preserves_source_and_releases_only_target_claim() {
    let temp = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(temp.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let context = SessionHomeContext {
        catalogue: Arc::new(FileSessionHomeCatalogue::with_store(
            layout.clone(),
            store.clone(),
        )),
        discovery: Arc::new(GitScopeDiscovery::default()),
        execution_dir: temp.path().into(),
    };
    let active = SessionIdentity::user_chat("chat-active").unwrap();
    let target = SessionIdentity::user_chat("chat-target").unwrap();
    let state = Arc::new(tokio::sync::RwLock::new(ActiveSessionState::new(
        active.clone(),
    )));
    let save = Arc::new(
        SaveSession::new(
            state.clone(),
            store.clone(),
            Arc::new(DurablePrefixLatch::default()),
            None,
            None,
            false,
        )
        .with_home(context.clone()),
    );
    let mut source = Session::new(target.clone());
    source.messages.push(Message::user("source"));
    store.save(&source).await.unwrap();
    std::fs::write(layout.home_file(&target), b"{broken").unwrap();
    let source_before = std::fs::read(layout.session_file(&target)).unwrap();
    store.release(&target);
    let resume = ResumeSavedSession::new(
        state.clone(),
        save,
        store.clone(),
        Arc::new(DepartingChildren::new(None)),
        false,
    )
    .with_home(context);
    let mut messages = vec![Message::user("active")];
    let before = messages.clone();
    let result = resume
        .execute(
            target.runtime_key(),
            &mut messages,
            None,
            &mut UntouchedRuntime,
        )
        .await;
    assert!(matches!(result, Err(crate::application::sessions::dto::ResumeSavedSessionError::Scope(crate::application::sessions::dto::resume_saved_session::ResumeDisposition::Unavailable(_)))));
    assert_eq!(messages.len(), before.len());
    assert_eq!(messages[0].content, before[0].content);
    assert_eq!(messages[0].id(), before[0].id());
    assert_eq!(state.read().await.identity(), &active);
    assert_eq!(
        std::fs::read(layout.session_file(&target)).unwrap(),
        source_before
    );
    assert_eq!(
        std::fs::read(layout.home_file(&target)).unwrap(),
        b"{broken"
    );
    let competitor = FileSessionStore::new(layout);
    competitor.claim(&target).unwrap();
    assert!(competitor.claim(&active).is_err());
}

#[tokio::test]
async fn existing_legacy_save_preserves_history_when_workspace_discovery_is_unavailable() {
    let temp = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(temp.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let identity = SessionIdentity::user_chat("chat-legacy-save").unwrap();
    let mut session = Session::new(identity.clone());
    session.messages.push(Message::user("before"));
    store.save(&session).await.unwrap();
    let context = SessionHomeContext {
        catalogue: Arc::new(FileSessionHomeCatalogue::with_store(
            layout.clone(),
            store.clone(),
        )),
        discovery: Arc::new(GitScopeDiscovery::default()),
        execution_dir: temp.path().join("missing"),
    };
    let state = Arc::new(tokio::sync::RwLock::new(ActiveSessionState::new(
        identity.clone(),
    )));
    let save = SaveSession::new(
        state,
        store.clone(),
        Arc::new(DurablePrefixLatch::default()),
        None,
        None,
        false,
    )
    .with_home(context.clone());
    session.messages.push(Message::user("after"));
    save.save(
        &mut session.messages,
        crate::application::sessions::dto::SaveTrigger::Routine,
    )
    .await
    .unwrap();
    assert_eq!(
        store.load(&identity).await.unwrap().unwrap().messages.len(),
        2
    );
    assert_eq!(
        context.catalogue.read(&identity).unwrap(),
        SessionHomeScope::LegacyUnscoped
    );
    assert!(!layout.home_file(&identity).exists());
}
