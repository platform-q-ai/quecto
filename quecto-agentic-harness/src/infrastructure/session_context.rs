//! Native runtime inspection for persisted session activation context.

use crate::domain::session::{
    CurrentFolderScope, FolderIdentity, FolderLabel, FolderScopeUnavailableReason,
    SessionContextInspector, SessionLocation, SessionPresentation,
};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

const IDENTITY_PREFIX: &str = "native_v1_unix_b64:";
const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub struct NativeSessionContextInspector {
    folder: PathBuf,
    agent_name: Option<String>,
}

impl NativeSessionContextInspector {
    pub fn new(folder: PathBuf, agent_name: Option<String>) -> Self {
        Self {
            folder,
            agent_name: agent_name.and_then(SessionPresentation::try_agent_name),
        }
    }

    pub fn current(agent_name: Option<String>) -> Result<Self, FolderScopeUnavailableReason> {
        std::env::current_dir()
            .map(|folder| Self::new(folder, agent_name))
            .map_err(|_| FolderScopeUnavailableReason::CanonicalizationFailed)
    }
}

impl SessionContextInspector for NativeSessionContextInspector {
    fn inspect_current(&self) -> CurrentFolderScope {
        resolve_native_folder(&self.folder, self.agent_name.clone())
    }
}

#[cfg(unix)]
pub fn resolve_native_folder(path: &Path, agent_name: Option<String>) -> CurrentFolderScope {
    let canonical = match std::fs::canonicalize(path) {
        Ok(path) => path,
        Err(_) => {
            return CurrentFolderScope::Unavailable(
                FolderScopeUnavailableReason::CanonicalizationFailed,
            );
        }
    };
    if !canonical.is_dir() {
        return CurrentFolderScope::Unavailable(FolderScopeUnavailableReason::NotADirectory);
    }
    let encoded = format!(
        "{IDENTITY_PREFIX}{}",
        encode_base64(canonical.as_os_str().as_bytes())
    );
    let identity = match FolderIdentity::try_from(encoded) {
        Ok(identity) => identity,
        Err(_) => {
            return CurrentFolderScope::Unavailable(FolderScopeUnavailableReason::IdentityTooLarge);
        }
    };
    let label = canonical
        .to_str()
        .and_then(|label| FolderLabel::try_from(label.to_owned()).ok());
    CurrentFolderScope::Known(SessionLocation::new(
        identity,
        label,
        agent_name.and_then(SessionPresentation::try_agent_name),
        inspect_branch(&canonical),
    ))
}

#[cfg(not(unix))]
pub fn resolve_native_folder(_path: &Path, _agent_name: Option<String>) -> CurrentFolderScope {
    CurrentFolderScope::Unavailable(FolderScopeUnavailableReason::UnsupportedPlatform)
}

fn inspect_branch(folder: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["symbolic-ref", "--short", "HEAD"])
        .current_dir(folder)
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() > 4096 || output.stderr.len() > 4096 {
        return None;
    }
    let branch = std::str::from_utf8(&output.stdout).ok()?.trim().to_owned();
    SessionPresentation::try_branch(branch)
}

fn encode_base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);
        out.push(B64[(a >> 2) as usize] as char);
        out.push(B64[(((a & 0x03) << 4) | (b >> 4)) as usize] as char);
        out.push(if chunk.len() > 1 {
            B64[(((b & 0x0f) << 2) | (c >> 6)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[(c & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn canonical_aliases_resolve_to_one_identity() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("target");
        std::fs::create_dir(&target).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, temp.path().join("alias")).unwrap();
        let direct = resolve_native_folder(&target, None);
        let alias = resolve_native_folder(&temp.path().join("alias"), None);
        assert_eq!(direct, alias);
    }

    #[test]
    fn non_directory_is_explicitly_unavailable() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("file");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(
            resolve_native_folder(&file, None),
            CurrentFolderScope::Unavailable(FolderScopeUnavailableReason::NotADirectory)
        );
    }
}
