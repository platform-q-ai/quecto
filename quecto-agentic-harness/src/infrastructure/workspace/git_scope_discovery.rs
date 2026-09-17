use super::filesystem_scope::FilesystemScope;
use crate::{
    application::sessions::ports::session_home::WorkspaceDiscovery,
    domain::{
        error::DomainError,
        session_home::{AssociationProvenance, SessionHome, WorkspaceGroup},
    },
};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct GitScopeDiscovery {
    /// Resolved once on the parent's PATH so every spawn is an absolute-program
    /// `posix_spawn`, never a fork of a tokio worker holding locked descriptors.
    executable: OsString,
}
impl Default for GitScopeDiscovery {
    fn default() -> Self {
        Self {
            executable: resolve_on_path("git")
                .map_or_else(|| "git".into(), PathBuf::into_os_string),
        }
    }
}

/// The first executable regular file named `program` on the parent's PATH.
fn resolve_on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(program))
        .find(|candidate| {
            std::fs::metadata(candidate).is_ok_and(|metadata| {
                use std::os::unix::fs::PermissionsExt;
                metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
            })
        })
}

/// Git reports "not a repository" with exit 128 and one of two prefixes: the
/// parent walk ended at the root, or at a filesystem boundary (tmpfs `/tmp`,
/// a separate `/home`, containers). Anything else stays an observable failure.
fn is_not_a_repository(status: &std::process::ExitStatus, stderr: &[u8]) -> bool {
    status.code() == Some(128) && stderr.starts_with(b"fatal: not a git repository")
}

impl WorkspaceDiscovery for GitScopeDiscovery {
    fn discover_async(
        &self,
        path: &Path,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<SessionHome, DomainError>> + Send + '_>,
    > {
        let discovery = self.clone();
        let path = path.to_path_buf();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || discovery.discover(&path))
                .await
                .map_err(|error| DomainError::Session(format!("Git discovery aborted: {error}")))?
        })
    }

    fn discover(&self, path: &Path) -> Result<SessionHome, DomainError> {
        let execution_dir = FilesystemScope.canonicalize(path)?;
        let output = self
            .command(&execution_dir)
            .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
            .output()
            .map_err(|error| DomainError::Session(format!("Git discovery unavailable: {error}")))?;
        let group = if output.status.success() {
            let root_output = self
                .command(&execution_dir)
                .args(["rev-parse", "--show-toplevel"])
                .output()
                .map_err(|error| {
                    DomainError::Session(format!("Git worktree unavailable: {error}"))
                })?;
            if root_output.status.success() {
                let root = FilesystemScope.canonicalize(&decode_path(&root_output.stdout)?)?;
                self.validate_membership(&execution_dir, &root)?;
                if execution_dir.starts_with(&root) {
                    for ancestor in execution_dir
                        .ancestors()
                        .take_while(|ancestor| *ancestor != root)
                    {
                        verify_no_marker(ancestor)?;
                    }
                } else {
                    return Err(DomainError::Session(
                        "Git worktree does not contain execution directory".into(),
                    ));
                }
            } else {
                return Err(DomainError::Session("Git working tree unavailable".into()));
            }
            let common_dir = decode_path(&output.stdout)?;
            WorkspaceGroup::Git {
                common_dir: FilesystemScope.canonicalize(&common_dir)?,
            }
        } else if is_not_a_repository(&output.status, &output.stderr) {
            verify_no_git_marker(&execution_dir)?;
            WorkspaceGroup::Folder {
                directory: execution_dir.clone(),
            }
        } else {
            return Err(DomainError::Session(format!(
                "Git discovery failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        };
        Ok(SessionHome {
            execution_dir,
            group,
            provenance: AssociationProvenance::SavedHere,
        })
    }
}

impl GitScopeDiscovery {
    fn validate_membership(&self, directory: &Path, root: &Path) -> Result<(), DomainError> {
        let output = self
            .command(directory)
            .args(["worktree", "list", "--porcelain", "-z"])
            .output()
            .map_err(|error| {
                DomainError::Session(format!("Git worktree listing unavailable: {error}"))
            })?;
        if output.status.success() {
            validate_worktree_membership(&output.stdout, root)
        } else {
            Err(DomainError::Session("Git worktree listing failed".into()))
        }
    }

    fn command(&self, directory: &Path) -> std::process::Command {
        let mut command = std::process::Command::new(&self.executable);
        // Only explicit ambient inputs are admitted: inherited GIT_DIR/GIT_WORK_TREE,
        // config injection and discovery ceilings must not redefine the requested scope.
        command.env_clear();
        for key in ["PATH", "SYSTEMROOT", "WINDIR"] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .env("LC_ALL", "C")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .current_dir(directory);
        command
    }
}

fn verify_no_git_marker(directory: &Path) -> Result<(), DomainError> {
    for ancestor in directory.ancestors() {
        verify_no_marker(ancestor)?;
    }
    Ok(())
}

fn verify_no_marker(ancestor: &Path) -> Result<(), DomainError> {
    match std::fs::symlink_metadata(ancestor.join(".git")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => {
            return Err(DomainError::Session(
                "Git metadata exists but discovery failed".into(),
            ));
        }
        Err(error) => {
            return Err(DomainError::Session(format!(
                "Git boundary unavailable: {error}"
            )));
        }
    }
    Ok(())
}

fn decode_path(bytes: &[u8]) -> Result<std::path::PathBuf, DomainError> {
    let bytes = bytes
        .strip_suffix(b"\n")
        .ok_or_else(|| DomainError::Session("Git returned an ambiguous path".into()))?;
    decode_raw_path(bytes)
}

fn decode_raw_path(bytes: &[u8]) -> Result<std::path::PathBuf, DomainError> {
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStringExt;
        std::path::PathBuf::from(OsString::from_vec(bytes.to_vec()))
    };
    #[cfg(not(unix))]
    let path = std::path::PathBuf::from(
        std::str::from_utf8(bytes)
            .map_err(|_| DomainError::Session("Git returned a non-Unicode path".into()))?,
    );
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(DomainError::Session(
            "Git returned a non-absolute path".into(),
        ))
    }
}

#[derive(PartialEq)]
enum WorktreeKind {
    WorkingTree,
    Bare,
}

fn validate_worktree_membership(bytes: &[u8], root: &Path) -> Result<(), DomainError> {
    let mut matches = 0;
    let mut record_root = None;
    let mut kind = WorktreeKind::WorkingTree;
    for field in bytes.split(|byte| *byte == 0) {
        if field.is_empty() {
            if let Some(path) = record_root.take() {
                if path == root && kind == WorktreeKind::WorkingTree {
                    matches += 1;
                }
            }
            kind = WorktreeKind::WorkingTree;
        } else if let Some(path) = field.strip_prefix(b"worktree ") {
            if record_root.is_none() {
                record_root = Some(decode_raw_path(path)?);
            } else {
                return Err(DomainError::Session("Ambiguous Git worktree record".into()));
            }
        } else if field == b"bare" {
            kind = WorktreeKind::Bare;
        } else if field.starts_with(b"HEAD ")
            || field.starts_with(b"branch ")
            || field == b"detached"
            || field.starts_with(b"locked")
            || field.starts_with(b"prunable")
        {
            // Known Git porcelain fields do not change scope identity.
        } else {
            return Err(DomainError::Session("Unknown Git worktree field".into()));
        }
    }
    if matches == 1 && bytes.ends_with(b"\0\0") {
        Ok(())
    } else {
        Err(DomainError::Session(
            "Git root has no unique non-bare worktree membership".into(),
        ))
    }
}

#[cfg(test)]
#[path = "git_scope_discovery_tests.rs"]
mod tests;
