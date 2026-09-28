//! The claude-code member handles the CLI holds (#2287). Declared here as
//! a plain struct of use-case handles; composition
//! (`composition::claude_member`) fills it. The interface never constructs
//! a use case or an adapter behind it.

use std::path::PathBuf;
use std::sync::Arc;

use crate::application::external_agent::use_cases::DriveExternalAgentSession;

/// What the agent CLI knows of the member it runs: composition maps it to
/// the session's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeMemberSettings {
    /// The member's session name (`-s`), naming its state directory.
    pub member: String,
    /// `--model`, when given.
    pub model: Option<String>,
    /// The checkout the member works in.
    pub checkout: PathBuf,
    /// quecto's base directory; the member's state lives beneath it.
    pub base_dir: PathBuf,
}

#[derive(Clone)]
pub struct ClaudeMemberHandles {
    /// The member's session: turns, steer and follow-up, abort, state.
    pub session: Arc<DriveExternalAgentSession>,
}

impl std::fmt::Debug for ClaudeMemberHandles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClaudeMemberHandles")
            .finish_non_exhaustive()
    }
}

/// Composition's builder of a claude-code member's handles (#2287),
/// injected through [`super::super::CliComposition`]: the handles, or why
/// the member cannot be built (a refusal to print).
pub type ClaudeMemberHandlesBuilder =
    fn(&ClaudeMemberSettings) -> Result<ClaudeMemberHandles, String>;
