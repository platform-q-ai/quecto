//! The scope disposition of a resume target (#2009): why the saved home
//! does not admit reuse of its history in the current runtime.
/// Minimum scope admission. Unimplemented actions never authorize history reuse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeDisposition {
    LegacyUnscoped,
    /// Same directory, but its workspace group changed since the save.
    HomeChanged,
    DifferentExecutionDirectory,
    Unavailable(String),
}

impl std::fmt::Display for ResumeDisposition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LegacyUnscoped => {
                f.write_str("legacy session requires explicit first association (unavailable)")
            }
            Self::HomeChanged => f.write_str(
                "session directory's workspace changed since it was saved (unavailable)",
            ),
            Self::DifferentExecutionDirectory => {
                f.write_str("session belongs to a different execution directory")
            }
            Self::Unavailable(_) => f.write_str(
                "session home or workspace is unavailable and needs validation or repair",
            ),
        }
    }
}
