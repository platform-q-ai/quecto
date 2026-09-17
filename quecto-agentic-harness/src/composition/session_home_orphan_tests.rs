//! Real-adapter acceptance (#2009 R2-H2): a home sidecar exists only while a
//! transcript does. A session that exits with nothing to save leaves no
//! durable metadata, so its name is not locked to a directory with no
//! history; an orphan sidecar found at startup is discarded, never
//! inherited by the first transcript written under the key.
use super::super::*;
use crate::application::durable_prefix::DurablePrefixLatch;
use crate::application::sessions::dto::SaveTrigger;
use crate::application::sessions::use_cases::{DepartingChildren, ResumeSavedSession, SaveSession};
use crate::application::sessions::{active_session::ActiveSessionState, ports::SessionStore};
use crate::domain::message::Message;
use crate::domain::session_home::SessionHomeScope;
use crate::domain::session_identity::SessionIdentity;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use std::path::{Path, PathBuf};

/// One loop over `store`, composed on `identity`, executing in `dir`.
struct Loop {
    save: Arc<SaveSession>,
    resume: ResumeSavedSession,
}

fn loop_in(identity: &SessionIdentity, store: Arc<FileSessionStore>, dir: &Path) -> Loop {
    let context = session_home_in(store.clone(), Ok(dir.to_path_buf()));
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
    let resume = ResumeSavedSession::new(
        state,
        save.clone(),
        store,
        Arc::new(DepartingChildren::new(None)),
        false,
        context,
    );
    Loop { save, resume }
}

fn sidecars(layout: &FlatSessionLayout) -> Vec<PathBuf> {
    match std::fs::read_dir(layout.sessions_dir()) {
        Ok(entries) => entries
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "home"))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// `-s work` in directory A, exit with zero messages: no `.home`, no
/// transcript. `-s work` in directory B then starts as a new session.
#[tokio::test]
async fn empty_exit_leaves_no_home_and_the_name_starts_in_another_directory() {
    let temp = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(temp.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let dir_a = temp.path().join("a");
    let dir_b = temp.path().join("b");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::create_dir_all(&dir_b).unwrap();
    let identity = SessionIdentity::named_cli("work").unwrap();

    let first = loop_in(&identity, store.clone(), &dir_a);
    first.resume.open_at_startup().await.expect("new session");
    first
        .save
        .save(&mut Vec::new(), SaveTrigger::OrdinaryExit)
        .await
        .expect("empty exit save");
    assert!(!layout.session_file(&identity).exists());
    assert!(
        !layout.home_file(&identity).exists(),
        "an empty session leaves no durable metadata"
    );
    assert_eq!(
        store.read_home(&identity).unwrap(),
        SessionHomeScope::LegacyUnscoped
    );
    store.release(&identity);

    let second = loop_in(&identity, store.clone(), &dir_b);
    let opened = second
        .resume
        .open_at_startup()
        .await
        .expect("the name is free to start elsewhere");
    assert!(opened.messages.is_empty());
    second
        .save
        .save(
            &mut vec![Message::user("hello from b")],
            SaveTrigger::Routine,
        )
        .await
        .unwrap();
    match store.read_home(&identity).unwrap() {
        SessionHomeScope::Scoped(home) => {
            assert_eq!(home.execution_dir, dir_b.canonicalize().unwrap());
        }
        other => panic!("the first transcript acquires its own home: {other:?}"),
    }
}

/// A sidecar without a transcript (a save that never committed, a transcript
/// removed by hand) is discarded at startup and is never inherited by the
/// first transcript written under the key.
#[tokio::test]
async fn orphan_home_is_discarded_at_startup_and_never_inherited() {
    let temp = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(temp.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let dir_a = temp.path().join("a");
    let dir_b = temp.path().join("b");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::create_dir_all(&dir_b).unwrap();
    let identity = SessionIdentity::named_cli("stale").unwrap();
    let home_a = session_home_in(store.clone(), Ok(dir_a.clone()))
        .current()
        .await
        .unwrap();
    store.record_new_home(&identity, &home_a).unwrap();
    store.release(&identity);
    assert!(layout.home_file(&identity).exists());
    assert!(!layout.session_file(&identity).exists());

    let started = loop_in(&identity, store.clone(), &dir_b);
    started
        .resume
        .open_at_startup()
        .await
        .expect("a home without a transcript is no session");
    assert!(
        !layout.home_file(&identity).exists(),
        "the orphan sidecar is discarded under the claim"
    );
    started
        .save
        .save(&mut vec![Message::user("first")], SaveTrigger::Routine)
        .await
        .unwrap();
    match store.read_home(&identity).unwrap() {
        SessionHomeScope::Scoped(home) => {
            assert_eq!(home.execution_dir, dir_b.canonicalize().unwrap());
        }
        other => panic!("the new transcript owns a fresh home: {other:?}"),
    }
}

/// A home beside a transcript is authority: startup admits it, and the
/// orphan rule never touches it.
#[tokio::test]
async fn a_home_beside_a_transcript_is_never_discarded() {
    let temp = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(temp.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let dir = temp.path().join("a");
    std::fs::create_dir_all(&dir).unwrap();
    let identity = SessionIdentity::named_cli("kept").unwrap();
    let first = loop_in(&identity, store.clone(), &dir);
    first.resume.open_at_startup().await.unwrap();
    first
        .save
        .save(&mut vec![Message::user("kept")], SaveTrigger::Routine)
        .await
        .unwrap();
    let bytes = std::fs::read(layout.home_file(&identity)).unwrap();
    store.release(&identity);
    store.discard_orphan_home(&identity).unwrap();
    assert_eq!(std::fs::read(layout.home_file(&identity)).unwrap(), bytes);
    store.release(&identity);
    let again = loop_in(&identity, store.clone(), &dir);
    let opened = again.resume.open_at_startup().await.unwrap();
    assert_eq!(opened.messages.len(), 1);
    assert_eq!(std::fs::read(layout.home_file(&identity)).unwrap(), bytes);
}

/// Every TUI tab opens a fresh `chat-*` key and most close with nothing
/// said: open/close leaves no `chat-*.home` litter behind.
#[tokio::test]
async fn tab_open_and_close_leaves_no_home_litter() {
    let temp = tempfile::tempdir().unwrap();
    let layout = FlatSessionLayout::new(temp.path());
    let store = Arc::new(FileSessionStore::new(layout.clone()));
    let dir = temp.path().join("workspace");
    std::fs::create_dir_all(&dir).unwrap();
    for tab in 0..3 {
        let identity = SessionIdentity::user_chat(&format!("chat-1700000000-tab{tab}")).unwrap();
        let opened = loop_in(&identity, store.clone(), &dir);
        opened.resume.open_at_startup().await.unwrap();
        // The loop's routine and exit saves both run with nothing to say.
        opened
            .save
            .save(&mut Vec::new(), SaveTrigger::Routine)
            .await
            .unwrap();
        opened
            .save
            .save(&mut Vec::new(), SaveTrigger::OrdinaryExit)
            .await
            .unwrap();
        store.release(&identity);
    }
    assert_eq!(sidecars(&layout), Vec::<PathBuf>::new());
    assert!(
        store
            .list(&crate::application::sessions::dto::SessionListQuery::All)
            .await
            .unwrap()
            .is_empty()
    );
}
