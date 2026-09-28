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
    check_credential(credential)?;
    if !member_dir.is_absolute() {
        return Err(MemberEnvironmentError::Directory {
            path: member_dir.to_path_buf(),
            detail: "the member directory must be an absolute path".into(),
        });
    }
    let home = private_dir(&member_dir.join(MEMBER_HOME_DIR))?;
    let config_dir = private_dir(&member_dir.join(MEMBER_CLAUDE_CONFIG_DIR))?;
    let mut variables: Vec<(String, OsString)> = INHERITED_VARIABLES
        .iter()
        .filter_map(|name| inherited(parent, name).map(|value| (name.to_string(), value)))
        .collect();
    variables.push(("HOME".into(), home.clone().into_os_string()));
    variables.push((
        "CLAUDE_CONFIG_DIR".into(),
        config_dir.clone().into_os_string(),
    ));
    variables.push((credential.name.clone(), OsString::from(&credential.value)));
    debug_assert!(
        variables.iter().all(|(name, _)| {
            INHERITED_VARIABLES.contains(&name.as_str())
                || CREDENTIAL_VARIABLES.contains(&name.as_str())
                || name == "HOME"
                || name == "CLAUDE_CONFIG_DIR"
        }),
        "only allowlisted names reach the member"
    );
    Ok(MemberEnvironment {
        variables,
        home,
        config_dir,
    })
}

/// A credential is given only under a known credential name, and only
/// with a value.
fn check_credential(credential: &CredentialEnv) -> Result<(), MemberEnvironmentError> {
    if !CREDENTIAL_VARIABLES.contains(&credential.name.as_str()) {
        return Err(MemberEnvironmentError::Credential(format!(
            "{} is not one of {CREDENTIAL_VARIABLES:?}",
            credential.name
        )));
    }
    if credential.value.is_empty() {
        return Err(MemberEnvironmentError::Credential(format!(
            "{} has no value",
            credential.name
        )));
    }
    Ok(())
}

/// The parent's value of `name` (the last one, as `getenv` would see it).
fn inherited(parent: &[(OsString, OsString)], name: &str) -> Option<OsString> {
    parent
        .iter()
        .rev()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, value)| value.clone())
}

/// Make `path` (and its missing parents) a real directory owned by this
/// user with mode [`PRIVATE_DIR_MODE`]; an existing one is narrowed to
/// it. A symlink, or anything but a directory, is refused.
fn private_dir(path: &Path) -> Result<PathBuf, MemberEnvironmentError> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let refuse = |detail: String| MemberEnvironmentError::Directory {
        path: path.to_path_buf(),
        detail,
    };
    match std::fs::DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_DIR_MODE)
        .create(path)
    {
        Ok(()) => {}
        Err(error) => return Err(refuse(format!("create: {error}"))),
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|e| refuse(format!("stat: {e}")))?;
    if !metadata.file_type().is_dir() {
        return Err(refuse("not a directory (a symlink is refused)".into()));
    }
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    let uid = unsafe { libc::geteuid() };
    if metadata.uid() != uid {
        return Err(refuse(format!(
            "owned by uid {}, not {uid}",
            metadata.uid()
        )));
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(PRIVATE_DIR_MODE))
        .map_err(|e| refuse(format!("chmod: {e}")))?;
    let mode = std::fs::symlink_metadata(path)
        .map_err(|e| refuse(format!("stat: {e}")))?
        .permissions()
        .mode()
        & 0o7777;
    if mode != PRIVATE_DIR_MODE {
        return Err(refuse(format!(
            "mode is {mode:o}, not {PRIVATE_DIR_MODE:o}"
        )));
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
#[path = "environment_tests.rs"]
mod tests;
