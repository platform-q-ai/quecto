//! Fork disposition: new opaque identity, transcript-only import (#2001 D4).

use std::sync::Arc;

use crate::application::sessions::dto::resume_disposition::{
    CrossFolderResumeRequest, DispositionError,
};
use crate::application::sessions::ports::resume_runtime::{DispositionApplyResult, ForkOutcome};
use crate::application::sessions::use_cases::resume_disposition::DecideResumeDisposition;
use crate::domain::session_home_scope::ResumeDisposition;

pub struct ForkSessionIntoScope {
    decide: Arc<DecideResumeDisposition>,
}

impl ForkSessionIntoScope {
    pub fn new(decide: Arc<DecideResumeDisposition>) -> Self {
        Self { decide }
    }

    pub fn execute(
        &self,
        request: &CrossFolderResumeRequest,
    ) -> Result<ForkOutcome, DispositionError> {
        match self
            .decide
            .execute(request, ResumeDisposition::ForkCurrent)?
        {
            DispositionApplyResult::Forked(outcome) => Ok(outcome),
            other => Err(DispositionError::TransactionFailed {
                detail: format!("expected Forked, got {other:?}"),
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
                new_identity: SessionIdentity::from_persisted_key("chat-new-opaque"),
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
    struct Tx {
        msgs: Vec<Message>,
    }
    impl TranscriptSource for Tx {
        fn load_messages(&self, _: &SessionIdentity) -> Result<Vec<Message>, DomainError> {
            Ok(self.msgs.clone())
        }
    }

    #[test]
    fn fork_wrapper_allocates_new_identity() {
        let decide = Arc::new(DecideResumeDisposition::new(
            Arc::new(OkOpen),
            Arc::new(OkFork),
            Arc::new(OkLocate),
            Arc::new(Tx {
                msgs: vec![Message::user("hi")],
            }),
        ));
        let uc = ForkSessionIntoScope::new(decide);
        let req = CrossFolderResumeRequest {
            selected: SessionIdentity::from_persisted_key("chat-src"),
            selected_home: SessionHomeScope::scoped(
                CanonicalExecutionLocation::from_canonical_path("/a"),
                None,
            ),
            current_location: CanonicalExecutionLocation::from_canonical_path("/b"),
            current_home: SessionHomeScope::scoped(
                CanonicalExecutionLocation::from_canonical_path("/b"),
                None,
            ),
            selected_home_reachable: true,
        };
        let out = uc.execute(&req).unwrap();
        assert_ne!(out.source, out.new_identity);
        assert_eq!(out.message_count, 1);
        assert!(!out.new_identity.runtime_key().contains('/'));
    }
}
