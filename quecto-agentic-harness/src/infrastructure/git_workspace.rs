//! Git CLI workspace discovery helpers (#2001 D2).
//! Pure relative to ports: shells out to `git` only; no domain I/O policy.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::domain::session_home_scope::{
    CanonicalExecutionLocation, RepositoryLabel, RepositoryWorktreeGrouping,
};

/// Result of probing Git for a working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitProbe {
    /// Inside a work tree with resolved roots and related worktrees.
    Inside {
        toplevel: CanonicalExecutionLocation,
        git_common_dir: CanonicalExecutionLocation,
        worktrees: Vec<CanonicalExecutionLocation>,
        label: RepositoryLabel,
    },
    /// Path is not inside any Git work tree.
    NotARepository,
    /// Git binary missing or failed in an unusable way.
    Unavailable { reason: String },
    /// Git responded but facts conflict or cannot be interpreted safely.
    Ambiguous { reason: String },
}

/// Runs Git discovery commands against a working directory.
#[derive(Debug, Default, Clone, Copy)]
pub struct GitCliProbe;

impl GitCliProbe {
    pub fn new() -> Self {
        Self
    }

    /// Probe `cwd` (must exist). Nested repositories win because `rev-parse`
    /// reports the nearest enclosing toplevel.
    pub fn probe(&self, cwd: &Path) -> GitProbe {
        match self.rev_parse_inside(cwd) {
            Ok(None) => GitProbe::NotARepository,
            Ok(Some((toplevel, common))) => match self.list_worktrees(cwd) {
                Ok(mut worktrees) => {
                    if worktrees.is_empty() {
                        worktrees.push(toplevel.clone());
                    }
                    let label = match repository_label_from_toplevel(toplevel.as_str()) {
                        Ok(l) => l,
                        Err(reason) => {
                            return GitProbe::Ambiguous { reason };
                        }
                    };
                    GitProbe::Inside {
                        toplevel,
                        git_common_dir: common,
                        worktrees,
                        label,
                    }
                }
                Err(reason) if reason.contains("ambiguous") => GitProbe::Ambiguous { reason },
                Err(reason) => GitProbe::Unavailable { reason },
            },
            Err(reason) if is_not_a_repo_message(&reason) => GitProbe::NotARepository,
            Err(reason) if reason.contains("ambiguous") => GitProbe::Ambiguous { reason },
            Err(reason) => GitProbe::Unavailable { reason },
        }
    }

    fn rev_parse_inside(
        &self,
        cwd: &Path,
    ) -> Result<Option<(CanonicalExecutionLocation, CanonicalExecutionLocation)>, String> {
        let inside = git_output(cwd, &["rev-parse", "--is-inside-work-tree"])?;
        let inside = inside.trim();
        if inside != "true" {
            return Ok(None);
        }
        let toplevel = git_output(cwd, &["rev-parse", "--show-toplevel"])?;
        let common = git_output(cwd, &["rev-parse", "--path-format=absolute", "--git-common-dir"])
            .or_else(|_| git_output(cwd, &["rev-parse", "--git-common-dir"]))?;
        let toplevel = canonicalize_text(toplevel.trim())?;
        let common = canonicalize_text(common.trim())?;
        Ok(Some((toplevel, common)))
    }

    fn list_worktrees(&self, cwd: &Path) -> Result<Vec<CanonicalExecutionLocation>, String> {
        let out = git_output(cwd, &["worktree", "list", "--porcelain"])?;
        parse_worktree_porcelain(&out)
    }
}

fn git_output(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| format!("git unavailable: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if !stderr.is_empty() { stderr } else { stdout };
        return Err(if detail.is_empty() {
            format!("git {:?} failed", args)
        } else {
            detail
        });
    }
    String::from_utf8(output.stdout).map_err(|e| format!("git output not utf-8: {e}"))
}

fn is_not_a_repo_message(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("not a git repository") || lower.contains("not a git repo")
}

fn canonicalize_text(path: &str) -> Result<CanonicalExecutionLocation, String> {
    if path.is_empty() {
        return Err("empty git path".into());
    }
    let p = PathBuf::from(path);
    let absolute = if p.is_absolute() {
        std::fs::canonicalize(&p).unwrap_or(p)
    } else {
        std::fs::canonicalize(&p).map_err(|e| format!("canonicalize {path}: {e}"))?
    };
    CanonicalExecutionLocation::try_from_canonical_path(absolute.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

fn parse_worktree_porcelain(out: &str) -> Result<Vec<CanonicalExecutionLocation>, String> {
    let mut locations = Vec::new();
    for line in out.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            let loc = canonicalize_text(path.trim())?;
            if !locations.iter().any(|l: &CanonicalExecutionLocation| l == &loc) {
                locations.push(loc);
            }
        }
    }
    Ok(locations)
}

fn repository_label_from_toplevel(toplevel: &str) -> Result<RepositoryLabel, String> {
    let name = Path::new(toplevel)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    RepositoryLabel::new(name).map_err(|e| e.to_string())
}

/// Build grouping facts from a successful Git probe and the session execution location.
pub fn grouping_from_probe(
    probe: &GitProbe,
    execution: CanonicalExecutionLocation,
) -> Option<RepositoryWorktreeGrouping> {
    match probe {
        GitProbe::Inside {
            worktrees, label, ..
        } => Some(RepositoryWorktreeGrouping::related_worktrees(
            label.clone(),
            execution,
            worktrees.clone(),
        )),
        _ => None,
    }
}

#[cfg(test)]
#[path = "git_workspace_tests.rs"]
mod tests;
