//! Filesystem adapter of the configuration capability's
//! [`MainWorktreeLocator`] port: a linked worktree's main worktree read
//! from git's own metadata files, never by running git (no hooks, no
//! fsmonitor, no filters, no configuration of the user's in play).
//!
//! The chain is accepted only in the shape `git worktree add` registers,
//! and every link must vouch for the next: `<dir>/.git` is a regular file
//! holding `gitdir: <admin>`; `<admin>/commondir` names the common
//! directory, and `<admin>` must be registered there — canonically a
//! direct child of `<common>/worktrees`, which only whoever can write the
//! repository's `.git` creates (a pointer, back-link and `commondir`
//! fabricated in the checkout itself never borrow another repository);
//! `<admin>/gitdir` names `<dir>/.git` back (a registered pointer copied
//! anywhere else is not that worktree); the common directory is a main
//! worktree's `.git` — named `.git`, its `core.bare` explicitly false, no
//! `core.worktree`, no includes — and the main worktree is its parent
//! directory. Anything else is [`WorktreeLink::Unresolved`], so the caller
//! shares nothing.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::application::configuration::ports::{MainWorktreeLocator, WorktreeLink};

/// The largest metadata file read; git's pointer and config files are far
/// smaller, so a larger one is not git's.
const METADATA_LIMIT: u64 = 64 * 1024;

#[derive(Debug, Default, Clone, Copy)]
pub struct GitMetadataWorktreeLocator;

impl MainWorktreeLocator for GitMetadataWorktreeLocator {
    fn locate(&self, directory: &Path) -> WorktreeLink {
        match main_worktree_of(directory) {
            Ok(Some(main_worktree)) => WorktreeLink::Linked { main_worktree },
            Ok(None) => WorktreeLink::NotLinked,
            Err(reason) => WorktreeLink::Unresolved { reason },
        }
    }
}

/// `Ok(None)` when `directory` is not the root of a linked worktree.
fn main_worktree_of(directory: &Path) -> Result<Option<PathBuf>, String> {
    let dot_git = directory.join(".git");
    match std::fs::symlink_metadata(&dot_git) {
        Ok(entry) if entry.is_dir() => return Ok(None),
        Ok(entry) if entry.is_file() => {}
        Ok(_) => {
            return Err(format!(
                "{} is neither a directory nor a gitdir file",
                dot_git.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", dot_git.display())),
    }
    let pointer = read_metadata(&dot_git)?.ok_or_else(|| vanished(&dot_git))?;
    let named = gitdir_pointer(&pointer)
        .ok_or_else(|| format!("{} is not a gitdir pointer", dot_git.display()))?;
    let admin = real_directory(directory, named)?;
    // No `commondir`: the git directory is its own common directory (a
    // submodule, a `--separate-git-dir` repository), not a linked worktree.
    let Some(common) = read_metadata(&admin.join("commondir"))? else {
        return Ok(None);
    };
    let common = real_directory(&admin, single_line(&common, &admin.join("commondir"))?)?;
    let registry = common.join("worktrees");
    if admin.parent() != Some(registry.as_path()) {
        return Err(format!(
            "{} is not registered in {}",
            admin.display(),
            registry.display()
        ));
    }
    let back_file = admin.join("gitdir");
    let back = read_metadata(&back_file)?.ok_or_else(|| vanished(&back_file))?;
    let back = admin.join(single_line(&back, &back_file)?);
    let names_us = match (back.canonicalize(), dot_git.canonicalize()) {
        (Ok(back), Ok(ours)) => back == ours,
        _ => false,
    };
    if !names_us {
        return Err(format!(
            "{} does not name {} back",
            back_file.display(),
            dot_git.display()
        ));
    }
    if common.file_name() != Some(std::ffi::OsStr::new(".git")) {
        return Err(format!(
            "{} is not a main worktree's .git directory (a bare repository has no main worktree)",
            common.display()
        ));
    }
    let config_file = common.join("config");
    let config = read_metadata(&config_file)?.ok_or_else(|| vanished(&config_file))?;
    core_layout(&config, true).map_err(|why| format!("{}: {why}", config_file.display()))?;
    let worktree_config = common.join("config.worktree");
    if let Some(config) = read_metadata(&worktree_config)? {
        core_layout(&config, false)
            .map_err(|why| format!("{}: {why}", worktree_config.display()))?;
    }
    let main = common
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", common.display()))?
        .to_path_buf();
    match std::fs::symlink_metadata(&main) {
        Ok(entry) if entry.is_dir() => {}
        _ => return Err(format!("{} is not a directory", main.display())),
    }
    assert!(main.is_absolute(), "a canonical path is absolute");
    Ok(Some(main))
}

/// The path a `.git` file names: `gitdir: <path>` on one line.
fn gitdir_pointer(pointer: &str) -> Option<&str> {
    let mut lines = pointer.lines().filter(|line| !line.trim().is_empty());
    let named = lines.next()?.strip_prefix("gitdir:")?.trim();
    (lines.next().is_none() && !named.is_empty()).then_some(named)
}

/// A metadata file holding one path on one line.
fn single_line<'a>(content: &'a str, file: &Path) -> Result<&'a str, String> {
    let mut lines = content.lines().filter(|line| !line.trim().is_empty());
    match (lines.next(), lines.next()) {
        (Some(line), None) => Ok(line.trim()),
        _ => Err(format!("{} does not hold one path", file.display())),
    }
}

/// `named` resolved against `base`, canonical, and an existing directory.
fn real_directory(base: &Path, named: &str) -> Result<PathBuf, String> {
    let joined = base.join(named);
    match joined.canonicalize() {
        Ok(path) if path.is_dir() => Ok(path),
        Ok(path) => Err(format!("{} is not a directory", path.display())),
        Err(error) => Err(format!("{}: {error}", joined.display())),
    }
}

/// A metadata file's text: `None` when absent; an error when it is not a
/// regular, small, UTF-8 file. The open itself refuses a final symbolic
/// link and never blocks (a FIFO), and the opened handle is what is
/// checked, so nothing swapped in between a check and the read is read.
fn read_metadata(path: &Path) -> Result<Option<String>, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let opened = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path);
    let file = match opened {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let entry = file
        .metadata()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    if !entry.is_file() {
        return Err(format!("{} is not a regular file", path.display()));
    }
    if entry.len() > METADATA_LIMIT {
        return Err(format!(
            "{} is too large to be git metadata",
            path.display()
        ));
    }
    // Bounded even if the file grows after the check.
    let mut text = String::new();
    file.take(METADATA_LIMIT)
        .read_to_string(&mut text)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(Some(text))
}

fn vanished(path: &Path) -> String {
    format!("{} could not be read", path.display())
}

/// Whether a git config file keeps the main worktree where the common
/// directory's parent is: `core.bare` explicitly false (required in the
/// repository's `config`, optional in `config.worktree`), no
/// `core.worktree`, and no include whose content is not read here.
fn core_layout(config: &str, bare_required: bool) -> Result<(), String> {
    let mut section = String::new();
    let mut non_bare = false;
    for line in config.lines() {
        let line = line.trim();
        if let Some(header) = line.strip_prefix('[') {
            let (name, rest) = header.split_once(']').unwrap_or((header, ""));
            if !matches!(rest.trim().chars().next(), None | Some('#' | ';')) {
                return Err("a section header carries a setting on its line".into());
            }
            section = name.trim().to_ascii_lowercase();
            if section == "include" || section.starts_with("includeif") {
                return Err("includes other configuration files".into());
            }
            continue;
        }
        if section != "core" {
            continue;
        }
        let line = line.split(['#', ';']).next().unwrap_or_default();
        let (key, value) = match line.split_once('=') {
            Some((key, value)) => (key.trim().to_ascii_lowercase(), Some(value.trim())),
            None => (line.trim().to_ascii_lowercase(), None),
        };
        match key.as_str() {
            "worktree" => return Err("core.worktree relocates the main worktree".into()),
            "bare" => match value.map(str::to_ascii_lowercase).as_deref() {
                Some("false" | "no" | "off" | "0") => non_bare = true,
                _ => return Err("core.bare is not false (a bare repository)".into()),
            },
            _ => {}
        }
    }
    match non_bare || !bare_required {
        true => Ok(()),
        false => Err("core.bare is not set false".into()),
    }
}

#[cfg(test)]
#[path = "main_worktree_tests.rs"]
pub(crate) mod tests;
