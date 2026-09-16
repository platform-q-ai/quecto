//! Decide and apply cross-folder resume dispositions (#2001 D4).
//!
//! Same-scope delegates to ResumeSavedSession ownership path.
//! Open-original / fork / locate / cancel are explicit and never silent.

use std::sync::Arc;

use crate::application::sessions::dto::resume_disposition::{
    plan_disposition, refuse_silent_cross_workspace_restore, CrossFolderResumeRequest,
    DispositionError, DispositionPlan,
};
use crate::application::sessions::ports::resume_runtime::{
    DispositionApplyResult, ForkTranscriptPort, LocateHomePort, OpenOriginalRuntimeLauncher,
    TranscriptSource,
};
use crate::domain::session_home_scope::ResumeDisposition;
use crate::domain::session_identity::SessionIdentity;

/// Application owner of cross-folder disposition planning and apply.
pub struct DecideResumeDisposition {
    open_original: Arc<dyn OpenOriginalRuntimeLauncher>,
    fork: Arc<dyn ForkTranscriptPort>,
    locate: Arc<dyn LocateHomePort>,
    transcripts: Arc<dyn TranscriptSource>,
}

impl DecideResumeDisposition {
    pub fn new(
        open_original: Arc<dyn OpenOriginalRuntimeLauncher>,
        fork: Arc<dyn ForkTranscriptPort>,
        locate: Arc<dyn LocateHomePort>,
        transcripts: Arc<dyn TranscriptSource>,
    ) -> Self {
        Self {
            open_original,
            fork,
            locate,
            transcripts,
        }
    }

    /// Refuse silent restore, then plan from explicit choice.
    pub fn plan(
        &self,
        request: &CrossFolderResumeRequest,
        choice: ResumeDisposition,
    ) -> Result<DispositionPlan, DispositionError> {
        refuse_silent_cross_workspace_restore(&request.selected_home, &request.current_home)
            .or_else(|e| {
                // Same-scope choice is allowed only when same_scope; open/fork/locate/cancel ok.
                if matches!(
                    choice,
                    ResumeDisposition::OpenOriginal
                        | ResumeDisposition::ForkCurrent
                        | ResumeDisposition::Locate
                        | ResumeDisposition::Cancel
                ) {
                    Ok(())
                } else if request.is_same_scope() {
                    Ok(())
                } else {
                    Err(e)
                }
            })?;
        // When same-scope, silent refuse is Ok; when cross-folder, only explicit choices pass above.
        if !request.is_same_scope()
            && matches!(choice, ResumeDisposition::SameScope)
        {
            return Err(DispositionError::DispositionNotApplicable {
                disposition: ResumeDisposition::SameScope,
            });
        }
        if request.is_same_scope() && matches!(choice, ResumeDisposition::SameScope) {
            return plan_disposition(request, choice);
        }
        if !request.is_same_scope() {
            // Explicit path: do not call refuse again as hard error for open/fork/locate.
            return plan_disposition(request, choice);
        }
        plan_disposition(request, choice)
    }

    /// Apply a plan through ports. Atomicity: on failure after partial work,
    /// ports must report TransactionFailed (adapters roll back).
    pub fn apply(&self, plan: DispositionPlan) -> Result<DispositionApplyResult, DispositionError> {
        match plan {
            DispositionPlan::Cancelled => Ok(DispositionApplyResult::Cancelled),
            DispositionPlan::SameScopeResume { identity } => {
                Ok(DispositionApplyResult::SameScope { identity })
            }
            DispositionPlan::OpenOriginal {
                identity,
                target_root,
            } => {
                let launch = self
                    .open_original
                    .launch_at_root(&identity, &target_root)
                    .map_err(|e| DispositionError::TransactionFailed {
                        detail: e.to_string(),
                    })?;
                if !launch.launched_without_chdir {
                    return Err(DispositionError::TransactionFailed {
                        detail: "open-original must not chdir current process".into(),
                    });
                }
                Ok(DispositionApplyResult::Opened(launch))
            }
            DispositionPlan::ForkCurrent {
                source,
                transcript,
                new_home,
            } => {
                let messages = if transcript.is_empty() {
                    self.transcripts.load_messages(&source).map_err(|e| {
                        DispositionError::TransactionFailed {
                            detail: e.to_string(),
                        }
                    })?
                } else {
                    transcript
                };
                let outcome = self
                    .fork
                    .fork_transcript(&source, &messages, &new_home)
                    .map_err(|e| DispositionError::TransactionFailed {
                        detail: e.to_string(),
                    })?;
                // Opaque keys must differ and never be path-derived from home.
                if outcome.source == outcome.new_identity {
                    return Err(DispositionError::TransactionFailed {
                        detail: "fork must allocate a new opaque identity".into(),
                    });
                }
                if outcome.new_identity.runtime_key().contains('/') {
                    return Err(DispositionError::TransactionFailed {
                        detail: "fork identity must remain opaque (no path)".into(),
                    });
                }
                Ok(DispositionApplyResult::Forked(outcome))
            }
            DispositionPlan::LocateReassociate {
                identity,
                new_home,
                provenance: _,
            } => {
                let outcome = self
                    .locate
                    .reassociate(&identity, &new_home)
                    .map_err(|e| DispositionError::TransactionFailed {
                        detail: e.to_string(),
                    })?;
                if outcome.identity != identity {
                    return Err(DispositionError::TransactionFailed {
                        detail: "locate must keep the same opaque identity".into(),
                    });
                }
                Ok(DispositionApplyResult::Located(outcome))
            }
        }
    }

    /// Convenience: plan then apply.
    pub fn execute(
        &self,
        request: &CrossFolderResumeRequest,
        choice: ResumeDisposition,
    ) -> Result<DispositionApplyResult, DispositionError> {
        let plan = self.plan(request, choice)?;
        self.apply(plan)
    }
}

impl std::fmt::Debug for DecideResumeDisposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecideResumeDisposition")
            .finish_non_exhaustive()
    }
}

/// Guard used by same-scope resume path: never restore across workspaces silently.
pub fn assert_same_scope_or_refuse(
    selected: &SessionIdentity,
    selected_home: &crate::domain::session_home_scope::SessionHomeScope,
    current_home: &crate::domain::session_home_scope::SessionHomeScope,
) -> Result<(), DispositionError> {
    let _ = selected;
    refuse_silent_cross_workspace_restore(selected_home, current_home)
}

#[cfg(test)]
#[path = "resume_disposition_tests.rs"]
mod tests;
