//! Pure resume-decision values (#2011): the version token of a session's
//! authoritative home, the explicit actions a user may choose, the kinds of
//! obstacle a home can present, and the one affirmative table of which actions
//! each kind offers. No filesystem, Git, UI or process call lives here.
use super::session_home::{AssociationProvenance, SessionHome, SessionHomeScope, WorkspaceGroup};
use super::session_identity::SessionIdentity;
use super::stable_digest::Fnv1a;

/// Opaque version of one session's authoritative home metadata, bound to that
/// session's identity: two reads that yield the same token saw the same
/// authority of the same session, and no two sessions — two legacy records,
/// two sessions saved in one folder — ever share a token. A client echoes the
/// token it was shown and the transaction refuses a selection whose token is
/// stale, so a token can authorize a change of exactly the record it names.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct HomeVersion(String);

const VERSION_PREFIX: &str = "h1-";

impl HomeVersion {
    /// The version of `identity`'s home `scope`: a deterministic digest of
    /// the identity and of every authoritative fact, the legacy and
    /// uninterpretable states included.
    pub fn of(identity: &SessionIdentity, scope: &SessionHomeScope) -> Self {
        let mut digest = Fnv1a::default();
        digest.field(b"session");
        digest.field(identity.runtime_key().as_bytes());
        match scope {
            SessionHomeScope::LegacyUnscoped => digest.field(b"legacy"),
            // The category alone: the reason is a reader's wording, not a fact.
            SessionHomeScope::Unavailable(_) => digest.field(b"unavailable"),
            SessionHomeScope::Scoped(home) => digest.home(home),
        }
        Self(format!("{VERSION_PREFIX}{:016x}", digest.value()))
    }

    /// Admit a token a client sent: exactly the shape [`Self::of`] produces.
    pub fn parse(raw: &str) -> Option<Self> {
        let digits = raw.strip_prefix(VERSION_PREFIX)?;
        let admitted = digits.len() == 16
            && digits
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        admitted.then(|| Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Fnv1a {
    fn home(&mut self, home: &SessionHome) {
        self.field(b"scoped");
        self.field(home.execution_dir.as_os_str().as_encoded_bytes());
        let (kind, path) = match &home.group {
            WorkspaceGroup::Git { common_dir } => (&b"git"[..], common_dir),
            WorkspaceGroup::Folder { directory } => (&b"folder"[..], directory),
        };
        self.field(kind);
        self.field(path.as_os_str().as_encoded_bytes());
        // Spelled out: a renamed variant must not silently change every version.
        self.field(match home.provenance {
            AssociationProvenance::SavedHere => b"saved_here",
        });
    }
}

/// Why a saved session cannot simply be restored in this runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeDecisionKind {
    /// Its home is intact and is another execution directory.
    CrossFolder,
    /// Its home directory cannot be observed: missing, moved or inaccessible.
    HomeMissing,
    /// Its home directory is this one, but its workspace group changed.
    HomeChanged,
    /// Its home metadata is present and cannot be interpreted.
    HomeUnknown,
    /// It predates home metadata and was never associated with a folder.
    LegacyUnscoped,
}

impl ResumeDecisionKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::CrossFolder => "cross_folder",
            Self::HomeMissing => "home_missing",
            Self::HomeChanged => "home_changed",
            Self::HomeUnknown => "home_unknown",
            Self::LegacyUnscoped => "legacy_unscoped",
        }
    }

    pub fn refusal_code(self) -> &'static str {
        match self {
            Self::CrossFolder => "belongs_elsewhere",
            Self::HomeMissing => "home_missing",
            Self::HomeChanged => "home_changed",
            Self::HomeUnknown => "home_unknown",
            Self::LegacyUnscoped => "no_home_recorded",
        }
    }
}

impl std::fmt::Display for ResumeDecisionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::CrossFolder => "session belongs to a different execution directory",
            Self::HomeMissing => "session directory is missing, moved or inaccessible",
            Self::HomeChanged => "session directory's workspace changed since it was saved",
            Self::HomeUnknown => "session home metadata cannot be interpreted",
            Self::LegacyUnscoped => "session has no folder recorded, so it cannot be resumed here",
        })
    }
}

#[cfg(test)]
#[path = "resume_decision_tests.rs"]
mod tests;
