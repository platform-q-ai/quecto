use std::fs;
use std::path::{Path, PathBuf};

use crate::application::sessions::ports::scope_discovery::{ScopeDiscoveryOutcome, SessionScopeDiscovery};
use crate::domain::session_scope::{AssociationProvenance, CanonicalExecutionLocation, RepositoryGrouping, SessionHomeScope};

pub struct FilesystemGitScopeDiscovery;

impl SessionScopeDiscovery for FilesystemGitScopeDiscovery {
    fn discover(&self, directory: &Path) -> ScopeDiscoveryOutcome {
        let execution = match directory.canonicalize() {
            Ok(path) if path.is_dir() => path,
            Ok(_) => return unavailable("execution location is not a directory"),
            Err(error) => return unavailable(format!("cannot canonicalize execution location: {error}")),
        };
        let Some(execution_text) = execution.to_str() else { return unavailable("execution location is not UTF-8") };
        let location = CanonicalExecutionLocation::new(execution_text).expect("canonical path is non-empty");

        let grouping = match nearest_git_marker(&execution) {
            Ok(None) => None,
            Ok(Some(marker)) => match grouping_from_marker(&marker) {
                Ok(grouping) => Some(grouping),
                Err(reason) => return ScopeDiscoveryOutcome::Ambiguous { reason },
            },
            Err(reason) => return unavailable(reason),
        };
        ScopeDiscoveryOutcome::Discovered(SessionHomeScope::scoped(location, grouping, AssociationProvenance::Discovered))
    }
}

fn unavailable(reason: impl Into<String>) -> ScopeDiscoveryOutcome {
    ScopeDiscoveryOutcome::Unavailable { reason: reason.into() }
}

fn nearest_git_marker(start: &Path) -> Result<Option<PathBuf>, String> {
    for ancestor in start.ancestors() {
        let marker = ancestor.join(".git");
        match fs::symlink_metadata(&marker) {
            Ok(_) => return Ok(Some(marker)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("cannot inspect {}: {error}", marker.display())),
        }
    }
    Ok(None)
}

fn grouping_from_marker(marker: &Path) -> Result<RepositoryGrouping, String> {
    let metadata = fs::symlink_metadata(marker).map_err(|e| format!("cannot inspect Git marker: {e}"))?;
    let git_dir = if metadata.is_dir() {
        marker.canonicalize().map_err(|e| format!("cannot canonicalize Git directory: {e}"))?
    } else if metadata.is_file() {
        parse_gitdir_file(marker)?
    } else {
        return Err("Git marker must be a directory or regular gitdir file".into());
    };
    let common_file = git_dir.join("commondir");
    let common_dir = match fs::read_to_string(&common_file) {
        Ok(value) => resolve_relative(&git_dir, value.trim(), "common Git directory")?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => git_dir.clone(),
        Err(error) => return Err(format!("cannot read common Git directory: {error}")),
    };
    let git_text = git_dir.to_str().ok_or("Git directory is not UTF-8")?;
    let common_text = common_dir.to_str().ok_or("common Git directory is not UTF-8")?;
    RepositoryGrouping::new(git_text, common_text).map_err(str::to_owned)
}

fn parse_gitdir_file(marker: &Path) -> Result<PathBuf, String> {
    let value = fs::read_to_string(marker).map_err(|e| format!("cannot read gitdir file: {e}"))?;
    let target = value.strip_prefix("gitdir:").map(str::trim).filter(|v| !v.is_empty()).ok_or("Git marker has an invalid gitdir declaration")?;
    resolve_relative(marker.parent().expect(".git marker has parent"), target, "Git directory")
}

fn resolve_relative(base: &Path, value: &str, label: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    let joined = if path.is_absolute() { path.to_path_buf() } else { base.join(path) };
    joined.canonicalize().map_err(|e| format!("cannot canonicalize {label}: {e}"))
}

#[cfg(test)]
#[path = "session_scope_discovery_tests.rs"]
mod tests;
