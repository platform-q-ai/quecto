//! Workspace file enumeration for the `@files` autocomplete.
//!
//! Lives in the infrastructure layer because it shells out to `git` and walks
//! the filesystem. Prefers `git ls-files` (tracked + untracked-not-ignored) and
//! falls back to a bounded filesystem walk when git is unavailable.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

/// Cap on entries returned by either source, so a huge monorepo can't make the
/// per-keystroke fuzzy scan lag (applies to both the git and fs-walk paths).
pub const MAX_WORKSPACE_FILES: usize = 5000;

/// List workspace files relative to `cwd`: prefer git (tracked +
/// untracked-not-ignored); fall back to a bounded filesystem walk. Paths
/// containing control characters are dropped — a filename carrying ANSI/escape
/// bytes could rewrite the terminal when shown in the popup or inserted.
pub fn list_workspace_files(cwd: &Path) -> Vec<String> {
    if let Some(files) = git_files(cwd) {
        if !files.is_empty() {
            return files;
        }
    }
    fs_walk(cwd)
}

/// Hardened `git ls-files` arguments. Disables repo-config-defined programs
/// (`core.fsmonitor`, hooks) that git would otherwise execute from a possibly
/// untrusted repo in the cwd — without this, typing `@` could run code planted
/// in a malicious `.git/config`. `-c` overrides come before the subcommand.
fn git_ls_files_args(extra: &[&str]) -> Vec<String> {
    let mut args = vec![
        "-c".into(),
        "core.fsmonitor=".into(),
        "-c".into(),
        "core.hooksPath=/dev/null".into(),
        "ls-files".into(),
        "-z".into(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    args
}

fn run_git(cwd: &Path, args: &[String]) -> Option<Vec<u8>> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    out.status.success().then_some(out.stdout)
}

fn git_files(cwd: &Path) -> Option<Vec<String>> {
    let tracked = run_git(cwd, &git_ls_files_args(&[]))?;
    let others =
        run_git(cwd, &git_ls_files_args(&["--others", "--exclude-standard"])).unwrap_or_default();
    Some(parse_git_output(&tracked, &others))
}

/// Parse NUL-delimited `git ls-files` output into sorted, sanitized,
/// capped relative paths (deduped across both sources).
fn parse_git_output(tracked: &[u8], others: &[u8]) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    'outer: for out in [tracked, others] {
        for part in out.split(|b| *b == 0) {
            if part.is_empty() {
                continue;
            }
            let s = String::from_utf8_lossy(part).into_owned();
            if is_safe_path(&s) {
                set.insert(s);
                if set.len() >= MAX_WORKSPACE_FILES {
                    break 'outer;
                }
            }
        }
    }
    set.into_iter().collect()
}

/// Reject empty paths and any path containing a control character (C0 `< 0x20`,
/// DEL `0x7f`, or C1) — these could inject terminal escape sequences.
fn is_safe_path(path: &str) -> bool {
    !path.is_empty() && !path.chars().any(char::is_control)
}

fn fs_walk(root: &Path) -> Vec<String> {
    const SKIP: &[&str] = &[".git", "target", "node_modules", ".jj", "dist"];
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if out.len() >= MAX_WORKSPACE_FILES {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || SKIP.contains(&name.as_ref()) {
                continue;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() {
                if let Ok(rel) = path.strip_prefix(root) {
                    let rel = rel.to_string_lossy();
                    if is_safe_path(&rel) {
                        out.push(rel.into_owned());
                    }
                }
                if out.len() >= MAX_WORKSPACE_FILES {
                    break;
                }
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
#[path = "workspace_files_tests.rs"]
mod tests;
