//! Contract for [`EndedChildRecords`] (#2192): the file adapter reads what
//! the child running as a session left — its crash record and its persisted
//! transcript — and nothing of another session, nor (#2192 review) a
//! record another process wrote when the pid of the child's is given.
//! An absent or malformed
//! crash record reads as `None`, never an error; an absent transcript is
//! `Ok(None)`.
use std::sync::Arc;

use quecto::application::sessions::ports::SessionStore;
use quecto::application::subagents::ports::EndedChildRecords;
use quecto::domain::crash_record::CrashRecord;
use quecto::domain::crash_record::PanicReport;
use quecto::domain::message::Message;
use quecto::domain::session::Session;
use quecto::domain::session_identity::SessionIdentity;
use quecto::infrastructure::persistence::audit_log::AuditLog;
use quecto::infrastructure::persistence::crash_record::Armed;
use quecto::infrastructure::persistence::ended_child_records::FileEndedChildRecords;

fn record() -> CrashRecord {
    CrashRecord::new(PanicReport::new("boom", Some("a.rs:1:2")), 9, 10).running(vec!["edit".into()])
}

fn port(base: &std::path::Path) -> (Arc<dyn EndedChildRecords>, Arc<dyn SessionStore>) {
    let store = Arc::new(quecto::composition::sessions::build_file_session_store(
        base,
    ));
    (
        Arc::new(FileEndedChildRecords::new(base, store.clone())),
        store,
    )
}

#[tokio::test]
async fn the_crash_record_of_the_named_session_is_read() {
    let base = tempfile::tempdir().unwrap();
    let (port, _) = port(base.path());
    let child = SessionIdentity::named_cli("child-1").unwrap();
    let other = SessionIdentity::named_cli("child-2").unwrap();
    assert_eq!(port.crash(&child, Some(9)).await, None, "no record yet");
    Armed::new(base.path(), Some(child.runtime_key()), None).record_fatal(&record(), "panic", 0);
    let written = record().for_session(child.runtime_key());
    assert_eq!(port.crash(&child, Some(9)).await, Some(written.clone()));
    assert_eq!(
        port.crash(&other, Some(9)).await,
        None,
        "another session's is not read"
    );
    assert_eq!(
        port.crash(&child, Some(8)).await,
        None,
        "another process's is not the child's"
    );
    assert_eq!(
        port.crash(&child, None).await,
        Some(written),
        "with no process expected, any is read"
    );
}

/// The child's record is read by its digest name, and only when it names
/// the child's own session: one naming another session under the child's
/// name is not the child's.
#[tokio::test]
async fn a_record_naming_another_session_is_not_the_childs() {
    let base = tempfile::tempdir().unwrap();
    let (port, _) = port(base.path());
    let child = SessionIdentity::named_cli("child-1").unwrap();
    let path = AuditLog::crash_directory(base.path())
        .join(AuditLog::crash_record_name(child.runtime_key()));
    assert!(
        !path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("child-1"),
        "named by the key's digest"
    );
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let foreign = serde_json::to_string(&record().for_session("cli:someone-else")).unwrap();
    std::fs::write(&path, foreign).unwrap();
    assert_eq!(port.crash(&child, Some(9)).await, None);
    assert_eq!(port.crash(&child, None).await, None);
}

#[tokio::test]
async fn a_malformed_crash_record_reads_as_none() {
    let base = tempfile::tempdir().unwrap();
    let (port, _) = port(base.path());
    let child = SessionIdentity::named_cli("child-1").unwrap();
    let path = AuditLog::crash_directory(base.path())
        .join(AuditLog::crash_record_name(child.runtime_key()));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{\"message\":").unwrap();
    assert_eq!(port.crash(&child, Some(9)).await, None);
    assert_eq!(port.crash(&child, None).await, None);
}

#[tokio::test]
async fn the_transcript_of_the_named_session_is_read() {
    let base = tempfile::tempdir().unwrap();
    let (port, store) = port(base.path());
    let child = SessionIdentity::named_cli("child-1").unwrap();
    assert!(port.transcript(&child).await.unwrap().is_none());
    let mut session = Session::new(child.clone());
    session.messages = vec![Message::user("before the crash")];
    store.save(&session).await.unwrap();
    let messages = port.transcript(&child).await.unwrap().unwrap();
    assert_eq!(messages.messages.len(), 1);
    assert_eq!(messages.messages[0].content, "before the crash");
    assert!(!messages.older_omitted);
    let other = SessionIdentity::named_cli("child-2").unwrap();
    assert!(port.transcript(&other).await.unwrap().is_none());
}

#[tokio::test]
async fn reading_a_childs_record_creates_nothing() {
    let base = tempfile::tempdir().unwrap();
    let (port, _) = port(base.path());
    let child = SessionIdentity::named_cli("child-1").unwrap();
    assert_eq!(port.crash(&child, None).await, None);
    assert!(
        !AuditLog::directory(base.path()).exists(),
        "a read leaves the base untouched"
    );
}
