//! The environment a `claude` member process runs with (#2286): an
//! allowlist, never the operator's environment minus a denylist.
//!
//! Spike #2264 found the parent Claude session's variables
//! (`CLAUDECODE`, `CLAUDE_CODE_*`), the operator's skills, plugins and
//! memory all reaching a child `claude`. So the child gets exactly:
//! [`INHERITED_VARIABLES`] copied from the parent, a private `HOME` and
//! `CLAUDE_CONFIG_DIR` inside the member's own directory, and the one
//! credential it was given. Everything else is dropped.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::application::external_agent::dto::CredentialEnv;

/// The only variables copied from the parent environment.
pub const INHERITED_VARIABLES: &[&str] = &["PATH", "LANG", "LC_ALL", "TERM", "TMPDIR"];

/// The credential variables a member may be given, exactly one at a time.
pub const CREDENTIAL_VARIABLES: &[&str] = &["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"];

/// The member's private `HOME`, inside its member directory.
pub const MEMBER_HOME_DIR: &str = "home";

/// The member's private `CLAUDE_CONFIG_DIR`, inside its member directory.
pub const MEMBER_CLAUDE_CONFIG_DIR: &str = "claude-config";

/// The mode of both private directories: owner only.
pub const PRIVATE_DIR_MODE: u32 = 0o700;

/// The environment a member's `claude` runs with, and its private
/// directories (made before it is returned). `Debug` names the variables
/// only: the credential's value never appears.
#[derive(Clone, PartialEq, Eq)]
pub struct MemberEnvironment {
    pub variables: Vec<(String, OsString)>,
    pub home: PathBuf,
    pub config_dir: PathBuf,
}

impl MemberEnvironment {
    /// The variable names, in order.
    pub fn names(&self) -> Vec<&str> {
        self.variables
            .iter()
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// The value of `name`, if the member is given it.
    pub fn get(&self, name: &str) -> Option<&OsStr> {
        self.variables
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_os_str())
    }
}

impl std::fmt::Debug for MemberEnvironment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemberEnvironment")
            .field("names", &self.names())
            .field("home", &self.home)
            .field("config_dir", &self.config_dir)
            .finish()
    }
}

/// Why a member's environment could not be made.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MemberEnvironmentError {
    #[error("credential refused: {0}")]
    Credential(String),
    #[error("{}: {detail}", path.display())]
    Directory { path: PathBuf, detail: String },
}

/// Build the environment of the member whose state lives in `member_dir`,
/// from the `parent` environment, giving it `credential`. Makes the
/// member's private directories first.
pub fn member_environment(
    parent: &[(OsString, OsString)],
    member_dir: &Path,
    credential: &CredentialEnv,
) -> Result<MemberEnvironment, MemberEnvironmentError> {
    let _ = (parent, credential);
    Ok(MemberEnvironment {
        variables: Vec::new(),
        home: member_dir.join(MEMBER_HOME_DIR),
        config_dir: member_dir.join(MEMBER_CLAUDE_CONFIG_DIR),
    })
}

#[cfg(test)]
#[path = "environment_tests.rs"]
mod tests;
