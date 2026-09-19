//! Pure resume-decision values (#2011): the version token of a session's
//! authoritative home, the explicit actions a user may choose, the kinds of
//! obstacle a home can present, and the one affirmative table of which actions
//! each kind offers. No filesystem, Git, UI or process call lives here.
use super::session_home::{SessionHome, SessionHomeScope, WorkspaceGroup};

/// Opaque version of a session's authoritative home metadata. Two reads that
/// yield the same token saw the same authority; a client echoes the token it
/// was shown and the transaction refuses a selection whose token is stale.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct HomeVersion(String);

const VERSION_PREFIX: &str = "h1-";

impl HomeVersion {
    /// The version of `scope`: a deterministic digest of every authoritative
    /// fact, the legacy and uninterpretable states included.
    pub fn of(scope: &SessionHomeScope) -> Self {
        let mut digest = Fnv1a::default();
        match scope {
            SessionHomeScope::LegacyUnscoped => digest.field(b"legacy"),
            SessionHomeScope::Unavailable(reason) => {
                digest.field(b"unavailable");
                digest.field(reason.as_bytes());
            }
            SessionHomeScope::Scoped(home) => digest.home(home),
        }
        Self(format!("{VERSION_PREFIX}{:016x}", digest.0))
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

/// FNV-1a over length-prefixed fields: stable across processes and releases,
/// unlike the standard hasher, and unambiguous across field boundaries.
struct Fnv1a(u64);

impl Default for Fnv1a {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv1a {
    fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn field(&mut self, bytes: &[u8]) {
        self.bytes(&(bytes.len() as u64).to_le_bytes());
        self.bytes(bytes);
    }

    fn home(&mut self, home: &SessionHome) {
        self.field(b"scoped");
        self.field(home.execution_dir.as_os_str().as_encoded_bytes());
        let (kind, path) = match &home.group {
            WorkspaceGroup::Git { common_dir } => (&b"git"[..], common_dir),
            WorkspaceGroup::Folder { directory } => (&b"folder"[..], directory),
        };
        self.field(kind);
        self.field(path.as_os_str().as_encoded_bytes());
        self.field(format!("{:?}", home.provenance).as_bytes());
    }
}

/// An explicit choice a user can make about a session that cannot simply be
/// restored here. `Cancel` is always executable; every other action needs an
/// executor composed into the runtime (#2012 open, #2013 fork, #2014 locate
/// and associate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResumeAction {
    OpenOriginal,
    ForkCurrent,
    Locate,
    Associate,
    Cancel,
}

impl ResumeAction {
    pub const ALL: [Self; 5] = [
        Self::OpenOriginal,
        Self::ForkCurrent,
        Self::Locate,
        Self::Associate,
        Self::Cancel,
    ];

    /// The stable name of the action on every boundary.
    pub fn name(self) -> &'static str {
        match self {
            Self::OpenOriginal => "open_original",
            Self::ForkCurrent => "fork_current",
            Self::Locate => "locate",
            Self::Associate => "associate",
            Self::Cancel => "cancel",
        }
    }

    /// Exact-name admission; nothing else denotes an action.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|action| action.name() == name)
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

    /// The affirmative table: the only actions this kind ever offers, in
    /// presentation order. None of them is a restore; `Cancel` is always last.
    pub fn offered_actions(self) -> &'static [ResumeAction] {
        use ResumeAction::{Associate, Cancel, ForkCurrent, Locate, OpenOriginal};
        match self {
            Self::CrossFolder => &[OpenOriginal, ForkCurrent, Cancel],
            Self::HomeMissing | Self::HomeChanged | Self::HomeUnknown => {
                &[Locate, ForkCurrent, Cancel]
            }
            Self::LegacyUnscoped => &[Associate, Cancel],
        }
    }

    pub fn offers(self, action: ResumeAction) -> bool {
        self.offered_actions().contains(&action)
    }
}

impl std::fmt::Display for ResumeDecisionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::CrossFolder => "session belongs to a different execution directory",
            Self::HomeMissing => "session directory is missing, moved or inaccessible",
            Self::HomeChanged => "session directory's workspace changed since it was saved",
            Self::HomeUnknown => "session home metadata cannot be interpreted",
            Self::LegacyUnscoped => "legacy session requires explicit first association",
        })
    }
}

#[cfg(test)]
#[path = "resume_decision_tests.rs"]
mod tests;
