//! Application DTOs for cross-folder resume dispositions (#2001 D4).
//!
//! Decision inputs and outcomes only — no filesystem, Git, or process I/O.
//! Domain [`ResumeDisposition`] is the wire/enum vocabulary; these DTOs
//! carry the transaction request/result shapes.

use crate::domain::error::DomainError;
use crate::domain::message::Message;
use crate::domain::session_home_scope::{
    AssociationProvenance, CanonicalExecutionLocation, ResumeDisposition, SessionHomeScope,
};
use crate::domain::session_identity::SessionIdentity;

/// Inputs for deciding how to resume a selected session relative to current scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossFolderResumeRequest {
    /// Opaque identity of the selected session (never path-derived).
    pub selected: SessionIdentity,
    /// Home scope recorded for the selected session (legacy-unscoped allowed).
    pub selected_home: SessionHomeScope,
    /// Canonical location of the current invocation / picker context.
    pub current_location: CanonicalExecutionLocation,
    /// Home scope discovered for the current invocation.
    pub current_home: SessionHomeScope,
    /// Whether the selected home's execution directory still exists (adapter fact).
    pub selected_home_reachable: bool,
}

impl CrossFolderResumeRequest {
    /// Affirmative: same-scope when both homes are scoped to equal locations.
    pub fn is_same_scope(&self) -> bool {
        self.selected_home.same_scope_as(&self.current_home)
            && self.selected_home.is_scoped()
            && self.current_home.is_scoped()
    }

    pub fn requires_cross_folder_choice(&self) -> bool {
        !self.is_same_scope() && self.selected_home.is_scoped()
    }

    pub fn selected_home_missing(&self) -> bool {
        self.selected_home.is_scoped() && !self.selected_home_reachable
    }
}

/// Outcome of a disposition decision before side effects run.
#[derive(Debug, Clone)]
pub enum DispositionPlan {
    /// Resume in place via existing ResumeSavedSession ownership path.
    SameScopeResume { identity: SessionIdentity },
    /// Launch a fresh runtime rooted at the original home (not chdir of current).
    OpenOriginal {
        identity: SessionIdentity,
        target_root: CanonicalExecutionLocation,
    },
    /// New opaque identity; import transcript messages only — no runtime state.
    ForkCurrent {
        source: SessionIdentity,
        /// Messages copied from the source transcript (authority).
        transcript: Vec<Message>,
        new_home: SessionHomeScope,
    },
    /// Explicit reassociation of the existing identity to current home.
    LocateReassociate {
        identity: SessionIdentity,
        new_home: SessionHomeScope,
        provenance: AssociationProvenance,
    },
    /// User cancelled; no mutation.
    Cancelled,
}

impl DispositionPlan {
    pub fn is_mutating(&self) -> bool {
        !matches!(self, Self::Cancelled)
    }

    pub fn disposition(&self) -> ResumeDisposition {
        match self {
            Self::SameScopeResume { .. } => ResumeDisposition::SameScope,
            Self::OpenOriginal { .. } => ResumeDisposition::OpenOriginal,
            Self::ForkCurrent { .. } => ResumeDisposition::ForkCurrent,
            Self::LocateReassociate { .. } => ResumeDisposition::Locate,
            Self::Cancelled => ResumeDisposition::Cancel,
        }
    }
}

/// Errors when planning or applying a cross-folder disposition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispositionError {
    /// Silent cross-workspace restore refused.
    CrossWorkspaceRestoreRefused,
    /// Disposition not allowlisted for this request shape.
    DispositionNotApplicable { disposition: ResumeDisposition },
    /// Selected home missing and user must choose Locate/Fork/Cancel (not silent).
    SelectedHomeMissing,
    /// Claim/create/save failed; caller must observe rollback.
    TransactionFailed { detail: String },
    /// Domain/application error surfaced.
    Domain(String),
}

impl From<DomainError> for DispositionError {
    fn from(e: DomainError) -> Self {
        Self::Domain(e.to_string())
    }
}

impl DispositionError {
    pub fn is_cross_workspace_refusal(&self) -> bool {
        matches!(self, Self::CrossWorkspaceRestoreRefused)
    }

    pub fn is_selected_home_missing(&self) -> bool {
        matches!(self, Self::SelectedHomeMissing)
    }
}

/// Plan a disposition from an explicit user choice (never guessed).
pub fn plan_disposition(
    request: &CrossFolderResumeRequest,
    choice: ResumeDisposition,
) -> Result<DispositionPlan, DispositionError> {
    match choice {
        ResumeDisposition::Cancel => Ok(DispositionPlan::Cancelled),
        ResumeDisposition::SameScope => {
            if request.is_same_scope() {
                Ok(DispositionPlan::SameScopeResume {
                    identity: request.selected.clone(),
                })
            } else {
                Err(DispositionError::DispositionNotApplicable {
                    disposition: ResumeDisposition::SameScope,
                })
            }
        }
        ResumeDisposition::OpenOriginal => {
            if request.selected_home_missing() {
                return Err(DispositionError::SelectedHomeMissing);
            }
            let target_root = request
                .selected_home
                .execution_location()
                .cloned()
                .ok_or(DispositionError::DispositionNotApplicable {
                    disposition: ResumeDisposition::OpenOriginal,
                })?;
            Ok(DispositionPlan::OpenOriginal {
                identity: request.selected.clone(),
                target_root,
            })
        }
        ResumeDisposition::ForkCurrent => {
            // Transcript filled by the use case from the store; plan carries empty until apply.
            Ok(DispositionPlan::ForkCurrent {
                source: request.selected.clone(),
                transcript: Vec::new(),
                new_home: request.current_home.clone(),
            })
        }
        ResumeDisposition::Locate => Ok(DispositionPlan::LocateReassociate {
            identity: request.selected.clone(),
            new_home: request.current_home.clone(),
            provenance: AssociationProvenance::LocateReassociation,
        }),
    }
}

/// Refuse any attempt to apply a scoped session into a different scope without an explicit plan.
pub fn refuse_silent_cross_workspace_restore(
    selected_home: &SessionHomeScope,
    current_home: &SessionHomeScope,
) -> Result<(), DispositionError> {
    if selected_home.is_legacy_unscoped() {
        return Ok(());
    }
    if selected_home.same_scope_as(current_home) {
        return Ok(());
    }
    Err(DispositionError::CrossWorkspaceRestoreRefused)
}

#[cfg(test)]
#[path = "resume_disposition_tests.rs"]
mod tests;
