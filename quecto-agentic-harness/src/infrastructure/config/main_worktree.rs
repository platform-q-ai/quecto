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
//! `<admin>/gitdir` names `<canonical dir>/.git` back, and the entry there
//! is the very file read (same device and inode, so a pointer swapped by a
//! concurrent local process after the read is refused; `.git` itself is
//! never canonicalised); a registered pointer copied anywhere else is not
//! that worktree. Registration is git's own: a worktree deleted without
//! `git worktree prune` stays registered, and a directory recreated at its
//! path with the same pointer is accepted, exactly as git accepts it; the
//! common directory is a main worktree's `.git` — named `.git`, its
//! `core.bare` explicitly false, no `core.worktree`, no includes — and the
//! main worktree is its parent directory. Anything else is
//! [`WorktreeLink::Unresolved`], so the caller shares nothing.

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
    main_worktree_with_seam(directory, || {})
}

/// [`main_worktree_of`], with `after_pointer_read` run right after the
/// `.git` pointer is read: the seam a test uses to swap the pointer the
/// way a concurrent local attacker would. Generic, so the release path is
/// monomorphised over the empty closure and carries no seam.
fn main_worktree_with_seam<F: FnMut()>(
    directory: &Path,
    mut after_pointer_read: F,
) -> Result<Option<PathBuf>, String> {
    // Canonical once, up front: the back-link must name `<root>/.git`
    // itself, and `.git` is never canonicalised (a link swapped in after
    // the read would be followed). A directory that no longer exists is no
    // worktree at all.
    let root = match directory.canonicalize() {
        Ok(root) => root,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", directory.display())),
    };
    let dot_git = root.join(".git");
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
    let (pointer, pointer_read) = read_identified(&dot_git)?.ok_or_else(|| vanished(&dot_git))?;
    after_pointer_read();
    let named = gitdir_pointer(&pointer)
        .ok_or_else(|| format!("{} is not a gitdir pointer", dot_git.display()))?;
    let admin = real_directory(&root, named)?;
    // No `commondir`: the git directory is its own common directory (a
    // submodule, a `--separate-git-dir` repository), not a linked worktree.
    let Some(common) = read_metadata(&admin.join("commondir"))? else {
        return Ok(None);
    };
    let common = real_directory(&admin, single_line(&common, &admin.join("commondir"))?)?;
    let registry = common.join("worktrees");
    let Some(_registered) = admin.parent().filter(|parent| *parent == registry) else {
        return Err(format!(
            "{} is not registered in {}",
            admin.display(),
            registry.display()
        ));
    };
    let back_file = admin.join("gitdir");
    let back = read_metadata(&back_file)?.ok_or_else(|| vanished(&back_file))?;
    let back = admin.join(single_line(&back, &back_file)?);
    let Some(_names_us) = back.canonicalize().ok().filter(|back| *back == dot_git) else {
        return Err(format!(
            "{} does not name {} back",
            back_file.display(),
            dot_git.display()
        ));
    };
    // The path is ours; the entry there now must be the file read. An
    // entry that cannot be examined says so, never "changed".
    match std::fs::symlink_metadata(&dot_git) {
        Ok(entry) if FileIdentity::of(&entry) == pointer_read => {}
        Ok(_) => {
            return Err(format!(
                "{} is not the file read (it changed while being resolved)",
                dot_git.display()
            ));
        }
        Err(error) => return Err(format!("{}: {error}", dot_git.display())),
    }
    let Some(main) = common
        .parent()
        .filter(|_| common.file_name() == Some(std::ffi::OsStr::new(".git")))
        .map(Path::to_path_buf)
    else {
        return Err(format!(
            "{} is not a main worktree's .git directory (a bare repository has no main worktree)",
            common.display()
        ));
    };
    let config_file = common.join("config");
    let config = read_metadata(&config_file)?.ok_or_else(|| vanished(&config_file))?;
    core_layout(&config, true).map_err(|why| format!("{}: {why}", config_file.display()))?;
    let worktree_config = common.join("config.worktree");
    if let Some(config) = read_metadata(&worktree_config)? {
        core_layout(&config, false)
            .map_err(|why| format!("{}: {why}", worktree_config.display()))?;
    }
    // The repository's own work tree: its `.git` is the common directory
    // every check above was made against (canonical, so never a link).
    assert_eq!(
        main.join(".git"),
        common,
        "the main worktree holds the common directory"
    );
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
    Ok(read_identified(path)?.map(|(text, _)| text))
}

/// The (device, inode) of a file, to tell the file read from whatever is
/// at its path later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    fn of(entry: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            device: entry.dev(),
            inode: entry.ino(),
        }
    }
}

/// [`read_metadata`], with the identity of the handle actually read.
fn read_identified(path: &Path) -> Result<Option<(String, FileIdentity)>, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let opened = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY)
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
    let identity = FileIdentity::of(&entry);
    let mut text = String::new();
    file.take(METADATA_LIMIT)
        .read_to_string(&mut text)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(Some((text, identity)))
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
