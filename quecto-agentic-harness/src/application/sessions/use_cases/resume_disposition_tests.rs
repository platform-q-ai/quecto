//! Behavioral unit tests for DecideResumeDisposition (#2001 D4).
//! Uses in-memory port doubles; intentionally asserts GREEN behavior.
//! RED is captured by temporarily stubbing apply ports to fail.

use super::*;
use crate::application::sessions::dto::resume_disposition::CrossFolderResumeRequest;
use crate::application::sessions::ports::resume_runtime::{
    ForkOutcome, LocateOutcome, OpenOriginalLaunch,
};
use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::session_home_scope::{
    CanonicalExecutionLocation, ResumeDisposition, SessionHomeScope,
};
use crate::domain::session_identity::SessionIdentity;
use std::sync::Mutex;

struct OkOpen;
impl OpenOriginalRuntimeLauncher for OkOpen {
    fn launch_at_root(
        &self,
        identity: &SessionIdentity,
        target_root: &CanonicalExecutionLocation,
    ) -> Result<OpenOriginalLaunch, DomainError> {
        Ok(OpenOriginalLaunch {
            identity: identity.clone(),
            runtime_root: target_root.clone(),
            launched_without_chdir: true,
        })
    }
}

struct ChdirOpen;
impl OpenOriginalRuntimeLauncher for ChdirOpen {
    fn launch_at_root(
        &self,
        identity: &SessionIdentity,
        target_root: &CanonicalExecutionLocation,
    ) -> Result<OpenOriginalLaunch, DomainError> {
        Ok(OpenOriginalLaunch {
            identity: identity.clone(),
            runtime_root: target_root.clone(),
            launched_without_chdir: false, // forbidden
        })
    }
}

struct OkFork {
    counter: Mutex<u64>,
}
impl ForkTranscriptPort for OkFork {
    fn fork_transcript(
        &self,
        source: &SessionIdentity,
        transcript: &[Message],
        new_home: &SessionHomeScope,
    ) -> Result<ForkOutcome, DomainError> {
        let n = {
            let mut c = self.counter.lock().unwrap();
            *c += 1;
            *c
        };
        Ok(ForkOutcome {
            source: source.clone(),
            new_identity: SessionIdentity::from_persisted_key(format!("chat-fork-{n}")),
            message_count: transcript.len(),
            new_home: new_home.clone(),
        })
    }
}

struct OkLocate;
impl LocateHomePort for OkLocate {
    fn reassociate(
        &self,
        identity: &SessionIdentity,
        new_home: &SessionHomeScope,
    ) -> Result<LocateOutcome, DomainError> {
        Ok(LocateOutcome {
            identity: identity.clone(),
            new_home: new_home.clone(),
        })
    }
}

struct MemTranscript {
    messages: Vec<Message>,
}
impl TranscriptSource for MemTranscript {
    fn load_messages(&self, _identity: &SessionIdentity) -> Result<Vec<Message>, DomainError> {
        Ok(self.messages.clone())
    }
}

fn scoped(p: &str) -> SessionHomeScope {
    SessionHomeScope::scoped(CanonicalExecutionLocation::from_canonical_path(p), None)
}

fn cross_req() -> CrossFolderResumeRequest {
    CrossFolderResumeRequest {
        selected: SessionIdentity::from_persisted_key("chat-sel"),
        selected_home: scoped("/orig"),
        current_location: CanonicalExecutionLocation::from_canonical_path("/cur"),
        current_home: scoped("/cur"),
        selected_home_reachable: true,
    }
}

fn uc_ok() -> DecideResumeDisposition {
    DecideResumeDisposition::new(
        Arc::new(OkOpen),
        Arc::new(OkFork {
            counter: Mutex::new(0),
        }),
        Arc::new(OkLocate),
        Arc::new(MemTranscript {
            messages: vec![Message::user("a"), Message::assistant("b", vec![])],
        }),
    )
}

#[test]
fn open_original_launches_without_chdir() {
    let uc = uc_ok();
    let result = uc
        .execute(&cross_req(), ResumeDisposition::OpenOriginal)
        .unwrap();
    match result {
        DispositionApplyResult::Opened(launch) => {
            assert!(launch.launched_without_chdir);
            assert_eq!(launch.runtime_root.as_str(), "/orig");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn open_original_rejects_chdir_style_launch() {
    let uc = DecideResumeDisposition::new(
        Arc::new(ChdirOpen),
        Arc::new(OkFork {
            counter: Mutex::new(0),
        }),
        Arc::new(OkLocate),
        Arc::new(MemTranscript { messages: vec![] }),
    );
    let err = uc
        .execute(&cross_req(), ResumeDisposition::OpenOriginal)
        .unwrap_err();
    assert!(matches!(err, DispositionError::TransactionFailed { .. }));
}

#[test]
fn fork_creates_new_opaque_identity_with_transcript_only() {
    let uc = uc_ok();
    let result = uc
        .execute(&cross_req(), ResumeDisposition::ForkCurrent)
        .unwrap();
    match result {
        DispositionApplyResult::Forked(outcome) => {
            assert_eq!(outcome.source.runtime_key(), "chat-sel");
            assert!(outcome.new_identity.runtime_key().starts_with("chat-fork-"));
            assert_eq!(outcome.message_count, 2);
            assert_eq!(
                outcome.new_home.execution_location().unwrap().as_str(),
                "/cur"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn locate_keeps_identity_and_reassociates_home() {
    let uc = uc_ok();
    let result = uc.execute(&cross_req(), ResumeDisposition::Locate).unwrap();
    match result {
        DispositionApplyResult::Located(outcome) => {
            assert_eq!(outcome.identity.runtime_key(), "chat-sel");
            assert_eq!(
                outcome.new_home.execution_location().unwrap().as_str(),
                "/cur"
            );
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn cancel_is_non_mutating() {
    let uc = uc_ok();
    let result = uc.execute(&cross_req(), ResumeDisposition::Cancel).unwrap();
    assert!(result.is_cancelled());
}

#[test]
fn same_scope_resume_plan_returns_identity_for_resume_saved_session() {
    let uc = uc_ok();
    let req = CrossFolderResumeRequest {
        selected: SessionIdentity::from_persisted_key("chat-sel"),
        selected_home: scoped("/same"),
        current_location: CanonicalExecutionLocation::from_canonical_path("/same"),
        current_home: scoped("/same"),
        selected_home_reachable: true,
    };
    let result = uc
        .execute(&req, ResumeDisposition::SameScope)
        .unwrap();
    match result {
        DispositionApplyResult::SameScope { identity } => {
            assert_eq!(identity.runtime_key(), "chat-sel");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn silent_cross_workspace_guard_blocks_without_explicit_choice_path() {
    let err = assert_same_scope_or_refuse(
        &SessionIdentity::from_persisted_key("chat-x"),
        &scoped("/a"),
        &scoped("/b"),
    )
    .unwrap_err();
    assert!(err.is_cross_workspace_refusal());
}

#[test]
fn missing_home_forces_locate_fork_or_cancel_not_open_original() {
    let uc = uc_ok();
    let mut req = cross_req();
    req.selected_home_reachable = false;
    let err = uc
        .execute(&req, ResumeDisposition::OpenOriginal)
        .unwrap_err();
    assert!(err.is_selected_home_missing());
    // Fork and cancel still work when home missing.
    uc.execute(&req, ResumeDisposition::ForkCurrent).unwrap();
    uc.execute(&req, ResumeDisposition::Cancel).unwrap();
    uc.execute(&req, ResumeDisposition::Locate).unwrap();
}
