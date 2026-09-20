//! Real-adapter acceptance over the composed home context: scope facts
//! never authorize cross-folder restore, and no home failure costs a
//! transcript. Lives in composition because it builds the real graph.
use super::*;
use crate::application::durable_prefix::DurablePrefixLatch;
use crate::application::sessions::dto::ResumeRequest;
use crate::application::sessions::dto::resume_saved_session::ResumeDisposition;
use crate::application::sessions::dto::{
    ListSessionsRequest, ResumeSavedSessionError, SaveTrigger, SessionListQuery, SessionListScope,
    StartupRefusal,
};
use crate::application::sessions::use_cases::{
    DepartingChildren, ListSessions, ResumeSavedSession, SaveSession,
};
use crate::application::sessions::{active_session::ActiveSessionState, ports::SessionStore};
use crate::domain::resume_decision::ResumeDecisionKind;
use crate::domain::session_home::SessionHomeScope;
use crate::domain::session_identity::SessionIdentity;
use crate::domain::{message::Message, session::Session};
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use std::path::{Path, PathBuf};

fn context(store: Arc<FileSessionStore>, execution_dir: PathBuf) -> SessionHomeContext {
    session_home_in(store, Ok(execution_dir))
}

fn state_of(
    identity: &SessionIdentity,
) -> crate::application::sessions::active_session::ActiveSessionHandle {
    Arc::new(tokio::sync::RwLock::new(ActiveSessionState::new(
        identity.clone(),
    )))
}

fn save_over(
    state: &crate::application::sessions::active_session::ActiveSessionHandle,
    store: Arc<FileSessionStore>,
    context: &SessionHomeContext,
) -> Arc<SaveSession> {
    Arc::new(
        SaveSession::new(
            state.clone(),
            store,
            Arc::new(DurablePrefixLatch::default()),
            None,
            None,
            false,
        )
        .with_home(context.clone()),
    )
}

fn resume_over(
    state: &crate::application::sessions::active_session::ActiveSessionHandle,
    save: Arc<SaveSession>,
    store: Arc<FileSessionStore>,
    context: SessionHomeContext,
) -> ResumeSavedSession {
    ResumeSavedSession::new(
        state.clone(),
        save,
        store,
        Arc::new(DepartingChildren::new(None)),
        false,
        context,
    )
}

fn git(cwd: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        // A hook-run test inherits GIT_DIR/GIT_WORK_TREE; they must not redirect the fixture.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn persisted_home_is_local_but_legacy_and_foreign_startup_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let here = temp.path().join("here");
    let elsewhere = temp.path().join("elsewhere");
    std::fs::create_dir(&here).unwrap();
    std::fs::create_dir(&elsewhere).unwrap();
    let store = Arc::new(FileSessionStore::new(FlatSessionLayout::new(temp.path())));
    let context = context(store.clone(), here);
    let identity = SessionIdentity::user_chat("chat-home").unwrap();
    let state = state_of(&identity);
    let save = save_over(&state, store.clone(), &context);
    save.save(
        &mut vec![Message::user("persisted here")],
        SaveTrigger::Routine,
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
        execution_dir: Ok(elsewhere),
        ..context.clone()
    };
    let resume = resume_over(&state, save.clone(), store.clone(), foreign);
    let refused = resume
        .open_at_startup()
        .await
        .expect_err("foreign directory");
    assert!(
        matches!(
            refused,
            ResumeSavedSessionError::StartupScope(StartupRefusal {
                disposition: ResumeDisposition::DifferentExecutionDirectory,
                execution_dir: Some(_),
                ..
            })
        ),
        "{refused:?}"
    );
    let said = refused.to_string();
    assert!(
        said.contains("\ncd '") && said.ends_with("then run the same command again"),
        "{said}"
    );
    assert!(
        !said.contains("quecto-tui"),
        "the reader ran quecto, not the TUI: {said}"
    );
    assert_eq!(state.read().await.identity(), &identity);
    let resume = resume_over(&state_of(&legacy), save, store, context);
    let refused = resume.open_at_startup().await.expect_err("legacy record");
    assert!(
        matches!(
            refused,
            ResumeSavedSessionError::StartupScope(StartupRefusal {
                disposition: ResumeDisposition::LegacyUnscoped,
                execution_dir: None,
                ..
            })
        ),
        "{refused:?}"
    );
    let text = refused.to_string();
    for expected in [
        "session 'chat-legacy' cannot start here",
        "it has no folder recorded",
        "saved transcript was not changed",
    ] {
        assert!(text.contains(expected), "{text}");
    }
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
    let context = context(store.clone(), temp.path().into());
    let active = SessionIdentity::user_chat("chat-active").unwrap();
    let target = SessionIdentity::user_chat("chat-target").unwrap();
    let state = state_of(&active);
    let save = save_over(&state, store.clone(), &context);
    let mut source = Session::new(target.clone());
    source.messages.push(Message::user("source"));
    store.save(&source).await.unwrap();
    std::fs::write(layout.home_file(&target), b"{broken").unwrap();
    let source_before = std::fs::read(layout.session_file(&target)).unwrap();
    store.release(&target);
    // The loop owns its session since startup; the refusal (#2011: decided
    // before the departing save) must leave that claim alone.
    store.claim(&active).unwrap();
    let resume = resume_over(&state, save, store.clone(), context);
    let mut messages = vec![Message::user("active")];
    let before = messages.clone();
    let result = resume
        .execute(
            &ResumeRequest::restore(target.runtime_key()),
            &mut messages,
            None,
            &mut UntouchedRuntime,
        )
        .await;
    assert!(matches!(
        result,
        Err(ResumeSavedSessionError::Decision(decision))
            if decision.kind == ResumeDecisionKind::HomeUnknown
    ));
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

/// Test gap (#2018 review): same Git group, different linked worktree,
/// selected by exact key — refused as a different execution directory.
/// Dropping the execution-directory compare from the domain rule (keeping
/// the group compare) fails here.
#[tokio::test]
async fn exact_key_resume_of_a_grouped_worktree_session_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(
        &repo,
        &[
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.test",
            "commit",
            "--allow-empty",
            "-qm",
            "init",
        ],
    );
    let linked = temp.path().join("linked");
    git(
        &repo,
        &["worktree", "add", "--detach", linked.to_str().unwrap()],
    );
    let base = temp.path().join("base");
    let store = Arc::new(FileSessionStore::new(FlatSessionLayout::new(&base)));
    let in_repo = context(store.clone(), repo.clone());
    let in_linked = context(store.clone(), linked.clone());
    let saved = SessionIdentity::user_chat("chat-in-repo").unwrap();
    let saved_state = state_of(&saved);
    save_over(&saved_state, store.clone(), &in_repo)
        .save(
            &mut vec![Message::user("repo history")],
            SaveTrigger::Routine,
        )
        .await
        .unwrap();
    store.release(&saved);
    let (SessionHomeScope::Scoped(saved_home), Ok(current)) = (
        in_linked.catalogue.read(&saved).unwrap(),
        in_linked.current().await,
    ) else {
        panic!("saved home and current facts are both observable")
    };
    assert_eq!(saved_home.group, current.group, "one Git group");
    assert_ne!(saved_home.execution_dir, current.execution_dir);
    let active = SessionIdentity::user_chat("chat-in-linked").unwrap();
    let active_state = state_of(&active);
    let save = save_over(&active_state, store.clone(), &in_linked);
    let resume = resume_over(&active_state, save, store.clone(), in_linked);
    let mut messages = vec![Message::user("linked")];
    let result = resume
        .execute(
            &ResumeRequest::restore(saved.runtime_key()),
            &mut messages,
            None,
            &mut UntouchedRuntime,
        )
        .await;
    assert!(
        matches!(
            &result,
            Err(ResumeSavedSessionError::Decision(decision))
                if decision.kind == ResumeDecisionKind::CrossFolder
        ),
        "{result:?}"
    );
    assert_eq!(messages[0].content, "linked");
    assert_eq!(active_state.read().await.identity(), &active);
    let competitor = FileSessionStore::new(FlatSessionLayout::new(&base));
    competitor.claim(&saved).unwrap();
}

/// L1: same directory whose group changed (a folder became a repository)
/// is `HomeChanged`, not a different execution directory.
#[tokio::test]
async fn a_folder_session_whose_directory_became_a_repository_is_home_changed() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("work");
    std::fs::create_dir(&dir).unwrap();
    let base = temp.path().join("base");
    let store = Arc::new(FileSessionStore::new(FlatSessionLayout::new(&base)));
    let context = context(store.clone(), dir.clone());
    let saved = SessionIdentity::user_chat("chat-folder").unwrap();
    let saved_state = state_of(&saved);
    save_over(&saved_state, store.clone(), &context)
        .save(&mut vec![Message::user("folder")], SaveTrigger::Routine)
        .await
        .unwrap();
    store.release(&saved);
    git(&dir, &["init", "-q"]);
    let scope = context.catalogue.read(&saved).unwrap();
    assert_eq!(
        context.admit(&scope).await,
        Err(ResumeDisposition::HomeChanged)
    );
    let active = SessionIdentity::user_chat("chat-now").unwrap();
    let active_state = state_of(&active);
    let save = save_over(&active_state, store.clone(), &context);
    let resume = resume_over(&active_state, save, store, context);
    let result = resume
        .execute(
            &ResumeRequest::restore(saved.runtime_key()),
            &mut vec![Message::user("now")],
            None,
            &mut UntouchedRuntime,
        )
        .await;
    assert!(
        matches!(
            &result,
            Err(ResumeSavedSessionError::Decision(decision))
                if decision.kind == ResumeDecisionKind::HomeChanged
        ),
        "{result:?}"
    );
}
