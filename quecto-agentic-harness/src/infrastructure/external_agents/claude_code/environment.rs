//! The environment a `claude` member process runs with (#2286): an
//! allowlist, never the operator's environment minus a denylist.
//!
//! Spike #2264 found the parent Claude session's variables
//! (`CLAUDECODE`, `CLAUDE_CODE_*`), the operator's skills, plugins and
//! memory all reaching a child `claude`. So the child gets exactly:
//! [`INHERITED_VARIABLES`] copied from the parent (the spike's login and
//! locale set, plus the proxy and CA variables a corporate network needs
//! to reach the API), a private `HOME` and `CLAUDE_CONFIG_DIR` inside the
//! member's own directory, and the one credential it was given. Everything
//! else is dropped.
//!
//! The member directory must be a real directory (not a symlink) owned by
//! this user that no one else can write: that is what keeps its entries
//! from being swapped under the launcher. Each private directory is made
//! and opened relative to the checked member directory's own descriptor
//! (`mkdirat`, `openat` with `O_NOFOLLOW | O_DIRECTORY`), then checked and
//! narrowed through that one open descriptor, so no check is ever about a
//! different file than the change.
//!
//! Decisions left to E2-S9 (#2293, credentials, config and the container
//! image):
//!
//! - **root + `bypassPermissions`.** `claude` refuses
//!   `--permission-mode bypassPermissions` when it runs as root unless
//!   `IS_SANDBOX=1` is set. `IS_SANDBOX` is not allowlisted here: a member
//!   running as root (a container image whose user is root) would fail to
//!   start. S9 decides between a non-root image user and giving
//!   `IS_SANDBOX=1` to container members only — as an allowlisted,
//!   launcher-set value, never inherited from the operator.
//! - **A mise shim as `claude`.** When the `claude` on `PATH` is a mise
//!   (or asdf) shim, the shim resolves the real binary through its config
//!   under `HOME`; under the member's private `HOME` it finds none and
//!   fails. S9 decides whether the launcher resolves the shim to its real
//!   binary before spawn, or the operator names the binary explicitly.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::application::external_agent::dto::CredentialEnv;

/// The only variables copied from the parent environment: locale, time
/// zone and login (as spike #2264 passed them), and the proxy and CA
/// settings in both the upper and lower case tools read.
pub const INHERITED_VARIABLES: &[&str] = &[
    "PATH",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "USER",
    "LOGNAME",
    "XDG_RUNTIME_DIR",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "ALL_PROXY",
    "all_proxy",
    "TZ",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NODE_EXTRA_CA_CERTS",
];

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
/// from the `parent` environment, giving it `credential`. Checks the
/// member directory and makes its private directories first.
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
    let member = member_directory(member_dir)?;
    let home = private_dir(&member, member_dir, MEMBER_HOME_DIR)?;
    let config_dir = private_dir(&member, member_dir, MEMBER_CLAUDE_CONFIG_DIR)?;
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
/// with a value. The launcher checks it with the rest of the spec, before
/// any directory is made.
pub fn check_credential(credential: &CredentialEnv) -> Result<(), MemberEnvironmentError> {
    let known = CREDENTIAL_VARIABLES.contains(&credential.name.as_str());
    if !known {
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

fn refusal(path: &Path) -> impl Fn(String) -> MemberEnvironmentError + '_ {
    move |detail| MemberEnvironmentError::Directory {
        path: path.to_path_buf(),
        detail,
    }
}

/// `path` opened as a directory without following a symlink at its last
/// component: a symlink, or anything but a directory, fails the open.
fn open_directory(path: &Path) -> Result<std::fs::File, MemberEnvironmentError> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| {
            refusal(path)(format!(
                "open as a directory, not following a symlink: {error}"
            ))
        })
}

/// The effective uid every directory must be owned by.
fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// Make `member_dir` (and its missing parents) if missing, then accept it
/// only as a real directory owned by this user that only its owner may
/// write. Its mode is otherwise left as it is. The checked directory's
/// descriptor is returned: its private directories are made through it.
fn member_directory(member_dir: &Path) -> Result<std::fs::File, MemberEnvironmentError> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    let refuse = refusal(member_dir);
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_DIR_MODE)
        .create(member_dir)
        .map_err(|error| refuse(format!("create: {error}")))?;
    let directory = open_directory(member_dir)?;
    let metadata = directory
        .metadata()
        .map_err(|error| refuse(format!("stat: {error}")))?;
    let uid = effective_uid();
    let owned = metadata.uid() == uid;
    let owner_only_writes = metadata.mode() & 0o022 == 0;
    if owned && owner_only_writes {
        return Ok(directory);
    }
    Err(refuse(format!(
        "must be owned by uid {uid} and writable by its owner only (uid {}, mode {:o})",
        metadata.uid(),
        metadata.mode() & 0o7777
    )))
}

/// A private directory's name: one plain path component.
fn plain_leaf(leaf: &str) -> bool {
    !leaf.is_empty()
        && leaf
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Make `leaf` a real directory owned by this user with mode
/// [`PRIVATE_DIR_MODE`] inside the already-checked member directory,
/// through `member`'s descriptor (`mkdirat`, `openat`) — never by a path
/// rebuilt from `member_dir`, which a swapped component could redirect.
/// An existing one is narrowed to the mode. The check and the narrowing
/// go through one `O_NOFOLLOW` descriptor, so a symlink swapped in is
/// never followed. Returns the directory's path, for the environment.
fn private_dir(
    member: &std::fs::File,
    member_dir: &Path,
    leaf: &str,
) -> Result<PathBuf, MemberEnvironmentError> {
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::MetadataExt;
    assert!(
        plain_leaf(leaf),
        "a private directory is one plain component"
    );
    let path = member_dir.join(leaf);
    let refuse = refusal(&path);
    let name = std::ffi::CString::new(leaf).expect("a plain leaf holds no NUL");
    // `member` is an open directory descriptor and `name` a NUL-terminated
    // string, both alive for the whole call; `mkdirat` only reads them.
    // SAFETY: valid descriptor and C string, neither kept by the call.
    let made = unsafe {
        libc::mkdirat(
            member.as_raw_fd(),
            name.as_ptr(),
            PRIVATE_DIR_MODE as libc::mode_t,
        )
    };
    let created = match made {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    };
    match created {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(refuse(format!("create: {error}"))),
    }
    // As for `mkdirat`; `openat` returns a new descriptor (or -1).
    // SAFETY: valid descriptor and C string, neither kept by the call.
    let fd = unsafe {
        libc::openat(
            member.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    let opened = fd >= 0;
    let directory = if opened {
        // `fd` was just returned by `openat`, is open, and nothing else
        // owns it: the `File` takes sole ownership and closes it.
        // SAFETY: a fresh, open, unowned descriptor.
        unsafe { std::fs::File::from_raw_fd(fd) }
    } else {
        let error = std::io::Error::last_os_error();
        return Err(refuse(format!(
            "open as a directory, not following a symlink: {error}"
        )));
    };
    let metadata = directory
        .metadata()
        .map_err(|error| refuse(format!("stat: {error}")))?;
    assert!(metadata.is_dir(), "an O_DIRECTORY open yields a directory");
    let uid = effective_uid();
    let owned = metadata.uid() == uid;
    if owned {
        return narrowed(&directory, &path);
    }
    Err(refuse(format!(
        "owned by uid {}, not {uid}",
        metadata.uid()
    )))
}

/// Narrow the open `directory` (one this user owns) to
/// [`PRIVATE_DIR_MODE`] and confirm it took. `File::set_permissions` is
/// `fchmod` on the open descriptor.
fn narrowed(directory: &std::fs::File, path: &Path) -> Result<PathBuf, MemberEnvironmentError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let refuse = refusal(path);
    directory
        .set_permissions(std::fs::Permissions::from_mode(PRIVATE_DIR_MODE))
        .map_err(|error| refuse(format!("chmod: {error}")))?;
    let mode = directory
        .metadata()
        .map_err(|error| refuse(format!("stat: {error}")))?
        .mode()
        & 0o7777;
    if mode == PRIVATE_DIR_MODE {
        return Ok(path.to_path_buf());
    }
    Err(refuse(format!(
        "mode is {mode:o}, not {PRIVATE_DIR_MODE:o}"
    )))
}

#[cfg(test)]
#[path = "environment_tests.rs"]
mod tests;
