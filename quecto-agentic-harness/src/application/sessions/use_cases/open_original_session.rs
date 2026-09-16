//! Open-original disposition: fresh target-rooted runtime, never chdir (#2001 D4).

use std::sync::Arc;

use crate::application::sessions::dto::resume_disposition::{
    CrossFolderResumeRequest, DispositionError,
};
use crate::application::sessions::ports::resume_runtime::{DispositionApplyResult, OpenOriginalLaunch};
use crate::application::sessions::use_cases::resume_disposition::DecideResumeDisposition;
use crate::domain::session_home_scope::ResumeDisposition;

/// Thin transaction wrapper around open-original.
pub struct OpenOriginalSession {
    decide: Arc<DecideResumeDisposition>,
}

impl OpenOriginalSession {
    pub fn new(decide: Arc<DecideResumeDisposition>) -> Self {
        Self { decide }
    }

    pub fn execute(
        &self,
        request: &CrossFolderResumeRequest,
    ) -> Result<OpenOriginalLaunch, DispositionError> {
        match self
            .decide
            .execute(request, ResumeDisposition::OpenOriginal)?
        {
            DispositionApplyResult::Opened(launch) => Ok(launch),
            other => Err(DispositionError::TransactionFailed {
                detail: format!("expected Opened, got {other:?}"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::sessions::dto::resume_disposition::CrossFolderResumeRequest;
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
                new_identity: SessionIdentity::from_persisted_key("chat-fork-1"),
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
    struct EmptyTx;
    impl TranscriptSource for EmptyTx {
        fn load_messages(&self, _: &SessionIdentity) -> Result<Vec<Message>, DomainError> {
            Ok(vec![])
        }
    }

    #[test]
    fn open_original_wrapper_returns_launch() {
        let decide = Arc::new(DecideResumeDisposition::new(
            Arc::new(OkOpen),
            Arc::new(OkFork),
            Arc::new(OkLocate),
            Arc::new(EmptyTx),
        ));
        let uc = OpenOriginalSession::new(decide);
        let req = CrossFolderResumeRequest {
            selected: SessionIdentity::from_persisted_key("chat-s"),
            selected_home: SessionHomeScope::scoped(
                CanonicalExecutionLocation::from_canonical_path("/orig"),
                None,
            ),
            current_location: CanonicalExecutionLocation::from_canonical_path("/cur"),
            current_home: SessionHomeScope::scoped(
                CanonicalExecutionLocation::from_canonical_path("/cur"),
                None,
            ),
            selected_home_reachable: true,
        };
        let launch = uc.execute(&req).unwrap();
        assert!(launch.launched_without_chdir);
        assert_eq!(launch.runtime_root.as_str(), "/orig");
    }
}
