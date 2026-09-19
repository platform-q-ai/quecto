//! Boundary values of a resume decision (#2011): what a client asks of a
//! saved session, which explicit actions this runtime can execute, and the
//! typed decision a home that does not admit a restore is answered with.
//! Domain values only — never wire JSON, never an adapter record.
use super::resume_saved_session::{ResumeTarget, SavedSessionResumed};
use crate::domain::resume_decision::{HomeVersion, ResumeAction, ResumeDecisionKind};
use std::collections::BTreeSet;
use std::path::PathBuf;

/// What the client wants done with the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeIntent {
    /// Restore its history here — admitted only for the same execution scope.
    Restore,
    /// One explicit action of a decision the client was shown.
    Act(ResumeAction),
}

/// A resume request: the target as the client spelled it, the intent, and the
/// home version the client was shown (a stale one authorizes nothing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeRequest {
    pub target: String,
    pub intent: ResumeIntent,
    pub expected_home_version: Option<HomeVersion>,
}

impl ResumeRequest {
    /// The exact-key restore: `/resume <key>` as typed.
    pub fn restore(target: &str) -> Self {
        Self {
            target: target.to_string(),
            intent: ResumeIntent::Restore,
            expected_home_version: None,
        }
    }
}

/// Whether this runtime can execute an offered action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionAvailability {
    Available,
    /// Not executable here; the reason is for the user.
    Unavailable(String),
}

/// The actions whose executor is composed into this runtime: an affirmative
/// set, empty but for `Cancel` until the executors' slices (#2012 open
/// original, #2013 fork, #2014 locate/associate) add theirs in composition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeActionCapabilities {
    executable: BTreeSet<ResumeAction>,
}

impl ResumeActionCapabilities {
    /// No executor is composed: every decision can only be cancelled.
    pub fn cancel_only() -> Self {
        Self {
            executable: BTreeSet::from([ResumeAction::Cancel]),
        }
    }

    /// Declare that `action`'s executor is composed into this runtime.
    #[must_use]
    pub fn with(mut self, action: ResumeAction) -> Self {
        self.executable.insert(action);
        self
    }

    pub fn availability(&self, action: ResumeAction) -> ActionAvailability {
        if action == ResumeAction::Cancel || self.executable.contains(&action) {
            ActionAvailability::Available
        } else {
            ActionAvailability::Unavailable(unavailable_reason(action).to_string())
        }
    }
}

/// What the user can do instead, while the executor is not delivered.
fn unavailable_reason(action: ResumeAction) -> &'static str {
    match action {
        ResumeAction::OpenOriginal => {
            "opening the original folder in a fresh runtime is not available yet; \
             start quecto in that folder to continue this session"
        }
        ResumeAction::ForkCurrent => {
            "forking the transcript into the current folder is not available yet"
        }
        ResumeAction::Locate => {
            "locating a moved session folder is not available yet; \
             the transcript is preserved"
        }
        ResumeAction::Associate => {
            "explicit association of a legacy session with a folder is not available yet; \
             start a new session with `-s <name>` — the old transcript stays in \
             place and visible under All Folders"
        }
        ResumeAction::Cancel => "cancel is always available",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeActionOffer {
    pub action: ResumeAction,
    pub availability: ActionAvailability,
}

/// The target exists and cannot simply be restored here: the obstacle, the
/// authority version it was decided on, and the explicit actions it offers.
/// Nothing was claimed, associated or restored to produce it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeDecision {
    pub target: ResumeTarget,
    pub kind: ResumeDecisionKind,
    pub home_version: HomeVersion,
    /// The saved execution directory, when the metadata names one. Untrusted.
    pub execution_dir: Option<PathBuf>,
    /// Why the home could not be observed or interpreted. Untrusted.
    pub detail: Option<String>,
    pub offers: Vec<ResumeActionOffer>,
}

impl ResumeDecision {
    pub fn offer(&self, action: ResumeAction) -> Option<&ResumeActionOffer> {
        self.offers.iter().find(|offer| offer.action == action)
    }
}

impl std::fmt::Display for ResumeDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "session resume unavailable: {}; choose", self.kind)?;
        for (index, offer) in self.offers.iter().enumerate() {
            let separator = if index == 0 { " " } else { " / " };
            match &offer.availability {
                ActionAvailability::Available => {
                    write!(f, "{separator}{}", offer.action.name())?;
                }
                ActionAvailability::Unavailable(_) => {
                    write!(f, "{separator}{} (unavailable)", offer.action.name())?;
                }
            }
        }
        match self
            .offers
            .iter()
            .find_map(|offer| match &offer.availability {
                ActionAvailability::Unavailable(reason) => Some(reason),
                ActionAvailability::Available => None,
            }) {
            Some(reason) => write!(f, ". {reason}"),
            None => Ok(()),
        }
    }
}

/// What a resume request that was not refused yielded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeOutcome {
    Resumed(SavedSessionResumed),
    /// The client cancelled: nothing was settled, saved, claimed or restored.
    Cancelled {
        name: String,
    },
}

#[cfg(test)]
#[path = "resume_decision_tests.rs"]
mod tests;
