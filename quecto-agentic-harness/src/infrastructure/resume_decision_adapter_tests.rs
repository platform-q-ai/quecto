use super::{FileSessionScopeTransaction, FreshRuntimeLauncher};
use crate::application::sessions::ports::resume_transaction::AtomicResumePersistence;
use crate::domain::{
    session::Session,
    session_identity::SessionIdentity,
    session_scope::{
        AssociationProvenance, CanonicalExecutionLocation, SessionHomeScope, SessionScopeMetadata,
    },
};
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use std::path::PathBuf;

#[test]
fn unavailable_target_is_refused_before_process_spawn() {
    let launcher = FreshRuntimeLauncher::new(PathBuf::from("/definitely/missing/program"));
    let err = launcher
        .launch(&PathBuf::from("relative"), "cli:key")
        .expect_err("refused");
    assert_eq!(err, "target folder is not an available absolute directory");
}

#[tokio::test]
async fn atomic_adapter_persists_transcript_and_scope_in_one_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let adapter = FileSessionScopeTransaction::new(FlatSessionLayout::new(dir.path()));
    let session = Session::new(SessionIdentity::from_persisted_key("cli:atomic"));
    let scope = SessionScopeMetadata::current(SessionHomeScope::scoped(
        CanonicalExecutionLocation::new("/target").unwrap(),
        None,
        AssociationProvenance::ExplicitlyLocated,
    ));
    adapter.commit(&session, &scope).await.unwrap();
    let body = std::fs::read_to_string(dir.path().join("sessions/cli_atomic.json")).unwrap();
    assert!(body.contains("/target"));
    assert!(body.contains("cli:atomic"));
    assert!(!dir.path().join("sessions/cli_atomic.tmp").exists());
}

#[test]
fn duplicate_claim_is_idempotent_for_owner_and_refused_by_competitor() {
    let dir = tempfile::tempdir().unwrap();
    let first = FileSessionScopeTransaction::new(FlatSessionLayout::new(dir.path()));
    let second = FileSessionScopeTransaction::new(FlatSessionLayout::new(dir.path()));
    let key = SessionIdentity::from_persisted_key("cli:owned");
    first.claim(&key).unwrap();
    first.claim(&key).unwrap();
    assert!(second.claim(&key).is_err());
    first.release(&key);
    second.claim(&key).unwrap();
}

#[test]
fn rollback_removes_staged_authority_and_intent_without_touching_current_record() {
    let dir = tempfile::tempdir().unwrap();
    let adapter = FileSessionScopeTransaction::new(FlatSessionLayout::new(dir.path()));
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(sessions.join("cli_safe.json"), "current").unwrap();
    std::fs::write(sessions.join("cli_safe.tmp"), "unsafe stage").unwrap();
    std::fs::write(sessions.join("cli_safe.resume-intent"), "resume-v1").unwrap();
    let key = SessionIdentity::from_persisted_key("cli:safe");
    adapter.rollback(&key);
    assert_eq!(
        std::fs::read_to_string(sessions.join("cli_safe.json")).unwrap(),
        "current"
    );
    assert!(!sessions.join("cli_safe.tmp").exists());
    assert!(!sessions.join("cli_safe.resume-intent").exists());
}

#[test]
fn recovery_promotes_staged_authority_and_clears_intent() {
    let dir = tempfile::tempdir().unwrap();
    let adapter = FileSessionScopeTransaction::new(FlatSessionLayout::new(dir.path()));
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(sessions.join("cli_recover.tmp"), "recovered").unwrap();
    std::fs::write(sessions.join("cli_recover.resume-intent"), "resume-v1").unwrap();
    adapter
        .recover(&SessionIdentity::from_persisted_key("cli:recover"))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(sessions.join("cli_recover.json")).unwrap(),
        "recovered"
    );
    assert!(!sessions.join("cli_recover.resume-intent").exists());
}

#[test]
fn empty_key_is_refused_before_process_spawn() {
    let launcher = FreshRuntimeLauncher::new(PathBuf::from("/definitely/missing/program"));
    let err = launcher
        .launch(&std::env::temp_dir(), "")
        .expect_err("refused");
    assert_eq!(err, "session key must not be empty");
}
