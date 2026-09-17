//! The startup refusal of the loop's own composed session (#2009): the scope
//! disposition worded for the command line — it names the key and what the
//! user can do now, never the resume picker's Cancel.
use super::resume_saved_session::ResumeDisposition;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupRefusal {
    /// The key as composed (`cli:default`, `cli:<name>`, `chat-…`).
    pub key: String,
    pub disposition: ResumeDisposition,
}

impl std::fmt::Display for StartupRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let key = &self.key;
        match &self.disposition {
            ResumeDisposition::LegacyUnscoped => write!(
                f,
                "session '{key}' cannot start here: it predates workspace scoping and has \
                 no home, and history is never associated implicitly. Start a new \
                 session under another name with `-s <name>` (or `--no-session` for an \
                 ephemeral run); the old transcript stays in place and visible in the \
                 Global list of /resume. Explicit association of a legacy session with \
                 a folder arrives in a later slice (#2014)."
            ),
            disposition @ ResumeDisposition::DifferentExecutionDirectory => write!(
                f,
                "session '{key}' cannot start here: {disposition}. Start it from that \
                 directory, or start a new session under another name with `-s <name>`."
            ),
            disposition @ (ResumeDisposition::HomeChanged | ResumeDisposition::Unavailable(_)) => {
                write!(
                    f,
                    "session '{key}' cannot start here: {disposition}. Start a new session \
                     under another name with `-s <name>`; the transcript is preserved."
                )
            }
        }
    }
}
