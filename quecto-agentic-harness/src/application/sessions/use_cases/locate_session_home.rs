//! Locate disposition: explicit reassociation of opaque identity (#2001 D4).

use std::sync::Arc;

use crate::application::sessions::dto::resume_disposition::{
    CrossFolderResumeRequest, DispositionError,
};
use crate::application::sessions::ports::resume_runtime::{DispositionApplyResult, LocateOutcome};
use crate::application::sessions::use_cases::resume_disposition::DecideResumeDisposition;
use crate::domain::session_home_scope::ResumeDisposition;

pub struct LocateSessionHome {
    decide: Arc<DecideResumeDisposition>,
}

impl LocateSessionHome {
    pub fn new(decide: Arc<DecideResumeDisposition>) -> Self {
        Self { decide }
    }

    pub fn execute(
        &self,
        request: &CrossFolderResumeRequest,
    ) -> Result<LocateOutcome, DispositionError> {
        match self.decide.execute(request, ResumeDisposition::Locate)? {
            DispositionApplyResult::Located(outcome) => Ok(outcome),
            other => Err(DispositionError::TransactionFailed {
                detail: format!("expected Located, got {other:?}"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::sessions::ports::resume_runtime::{
        ForkOutcome, ForkTranscriptPort, LocateHomePort, LocateOutcome, OpenOriginalLaunch,
        OpenOriginalRuntimeLauncher, TranscriptSource,
    };
    use crate::domain::error::DomainError;
    use crate::domain::message::Message;
    use crate::domain::session_home_scope::{CanonicalExecutionLocation, SessionHomeScope};
    use crate::domain::session_identity::SessionIdentity;
    use std::sync::Mutex;

    /// In-memory locate port that rolls back on forced failure after claim.
    struct AtomicLocate {
        homes: Mutex<Vec<(String, SessionHomeScope)>>,
        fail_after_claim: bool,
    }
    impl LocateHomePort for AtomicLocate {
        fn reassociate(
            &self,
            identity: &SessionIdentity,
            new_home: &SessionHomeScope,
        ) -> Result<LocateOutcome, DomainError> {
            let mut g = self.homes.lock().unwrap();
            let prior = g
                .iter()
                .find(|(k, _)| k == identity.runtime_key())
                .map(|(_, h)| h.clone());
            g.retain(|(k, _)| k != identity.runtime_key());
            g.push((identity.runtime_key().to_string(), new_home.clone()));
            if self.fail_after_claim {
                // rollback
                g.retain(|(k, _)| k != identity.runtime_key());
                if let Some(p) = prior {
                    g.push((identity.runtime_key().to_string(), p));
                }
                return Err(DomainError::Session("forced fail after claim".into()));
            }
            Ok(LocateOutcome {
                identity: identity.clone(),
                new_home: new_home.clone(),
            })
        }
    }

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
    struct OkFork;
    impl ForkTranscriptPort for OkFork {
        fn fork_transcript(
            &self,
            source: &SessionIdentity,
            transcript: &[Message],
            new_home: &SessionHomeScope,
        ) -> Result<ForkOutcome, DomainError> {
            Ok(ForkOutcome {
                source: source.clone(),
                new_identity: SessionIdentity::from_persisted_key("chat-f"),
                message_count: transcript.len(),
                new_home: new_home.clone(),
            })
        }
    }
    struct EmptyTx;
    impl TranscriptSource for EmptyTx {
        fn load_messages(&self, _: &SessionIdentity) -> Result<Vec<Message>, DomainError> {
            Ok(vec![])
        }
    }

    fn req() -> CrossFolderResumeRequest {
        CrossFolderResumeRequest {
            selected: SessionIdentity::from_persisted_key("chat-loc"),
            selected_home: SessionHomeScope::scoped(
                CanonicalExecutionLocation::from_canonical_path("/old"),
                None,
            ),
            current_location: CanonicalExecutionLocation::from_canonical_path("/new"),
            current_home: SessionHomeScope::scoped(
                CanonicalExecutionLocation::from_canonical_path("/new"),
                None,
            ),
            selected_home_reachable: false,
        }
    }

    #[test]
    fn locate_success_reassociates() {
        let locate = Arc::new(AtomicLocate {
            homes: Mutex::new(vec![]),
            fail_after_claim: false,
        });
        let decide = Arc::new(DecideResumeDisposition::new(
            Arc::new(OkOpen),
            Arc::new(OkFork),
            locate.clone(),
            Arc::new(EmptyTx),
        ));
        let out = LocateSessionHome::new(decide).execute(&req()).unwrap();
        assert_eq!(out.identity.runtime_key(), "chat-loc");
        assert_eq!(out.new_home.execution_location().unwrap().as_str(), "/new");
    }

    #[test]
    fn locate_failure_rolls_back_home_binding() {
        let locate = Arc::new(AtomicLocate {
            homes: Mutex::new(vec![(
                "chat-loc".into(),
                SessionHomeScope::scoped(
                    CanonicalExecutionLocation::from_canonical_path("/old"),
                    None,
                ),
            )]),
            fail_after_claim: true,
        });
        let decide = Arc::new(DecideResumeDisposition::new(
            Arc::new(OkOpen),
            Arc::new(OkFork),
            locate.clone(),
            Arc::new(EmptyTx),
        ));
        let err = LocateSessionHome::new(decide).execute(&req()).unwrap_err();
        assert!(matches!(err, DispositionError::TransactionFailed { .. }));
        let homes = locate.homes.lock().unwrap();
        assert_eq!(homes.len(), 1);
        assert_eq!(homes[0].1.execution_location().unwrap().as_str(), "/old");
    }
}
