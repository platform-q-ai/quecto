//! What fd's lines become: entries of the kind asked for, shown relative to
//! the workspace (#2203), kept until one past the limit (#2200 review), and
//! the VCS metadata the search skipped (#2199).
use super::{FindEntryKind, VCS_METADATA_DIRS};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

/// Where fd searched, and how its entries are shown: relative to the
/// workspace, as grep shows them, or whole outside it (#2203).
pub(super) struct SearchRoot {
    pub(super) searched: PathBuf,
    /// Empty for the workspace itself, else a prefix ending in '/'.
    pub(super) shown: String,
    /// What fd's own relative lines are relative to (its working directory).
    pub(super) workspace: PathBuf,
    pub(super) kind: Option<FindEntryKind>,
    pub(super) skipped_vcs_dir: Option<String>,
}

impl SearchRoot {
    pub(super) fn new(searched: PathBuf, workspace: &Path, kind: Option<FindEntryKind>) -> Self {
        let shown = shown_prefix(&searched, workspace);
        assert!(
            shown.is_empty() || shown.ends_with('/'),
            "a shown root is empty or a directory prefix"
        );
        let skipped_vcs_dir = skipped_vcs_dir(&searched, workspace);
        Self {
            searched,
            shown,
            workspace: workspace.to_path_buf(),
            kind,
            skipped_vcs_dir,
        }
    }

    /// One fd record (raw bytes, as fd printed them) as shown, or `None`
    /// when it is not of the kind asked for. With a kind, fd lists symlinks
    /// as well, each kept by what it points at: under type d every record
    /// without '/' is a symlink and is followed; under type f each record
    /// is looked up without following, and only a symlink is then followed.
    /// A symlink is shown as fd shows it, without '/', whatever it points at.
    pub(super) fn entry(&self, record: &[u8]) -> Option<String> {
        // The name is looked up by its bytes, never by a lossy rendering.
        let path = self.workspace.join(OsStr::from_bytes(record));
        let kept = match (self.kind, record.ends_with(b"/")) {
            (None, _) => true,
            // fd marks only real directories with '/'.
            (Some(FindEntryKind::Directory), true) => true,
            (Some(FindEntryKind::Directory), false) => std::fs::metadata(&path).ok()?.is_dir(),
            (Some(FindEntryKind::File), false) => {
                let entry = std::fs::symlink_metadata(&path).ok()?;
                match (entry.is_file(), entry.file_type().is_symlink()) {
                    (true, _) => true,
                    (false, true) => std::fs::metadata(&path).ok()?.is_file(),
                    (false, false) => false,
                }
            }
            (Some(FindEntryKind::File), true) => false,
        };
        match kept {
            true => Some(self.shown_entry(record)),
            false => None,
        }
    }

    fn shown_entry(&self, record: &[u8]) -> String {
        // fd prints each entry under the root exactly as it was given.
        let searched = self.searched.as_os_str().as_bytes();
        let mut prefix = searched[..searched.len() - trailing_slashes(searched)].to_vec();
        prefix.push(b'/');
        match record.strip_prefix(prefix.as_slice()) {
            // A root given as `src//` must not show `src//a.rs`.
            Some(relative) => format!(
                "{}{}",
                self.shown,
                displayed(&relative[leading_slashes(relative)..])
            ),
            None => displayed(record),
        }
    }
}

fn trailing_slashes(bytes: &[u8]) -> usize {
    bytes.iter().rev().take_while(|byte| **byte == b'/').count()
}

fn leading_slashes(bytes: &[u8]) -> usize {
    bytes.iter().take_while(|byte| **byte == b'/').count()
}

/// A name as one line of text: bytes that are not UTF-8 become U+FFFD, and
/// control characters (a newline in a name) are escaped, so one entry is
/// always one line (#2200 review 3).
fn displayed(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .flat_map(|character| match character.is_control() {
            true => character.escape_default().collect::<Vec<_>>(),
            false => vec![character],
        })
        .collect()
}

/// Why reading fd's output stopped before EOF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stop {
    /// The kept entries reached the byte cap, or fd printed a record longer
    /// than any path: what was kept is an arbitrary subset.
    Capped,
    /// One past the limit is kept: more exist, and fd is not needed.
    Enough,
}

/// The longest record read: a path fd prints, with slack, is shorter.
pub(super) const RECORD_MAX: usize = 4 * libc::PATH_MAX as usize;

/// fd's stdout (`--print0` records), filtered as it arrives. The byte cap
/// is charged only for kept entries, so records of the other kind never
/// use it up (#2200 review 3); an unfinished record is bounded by
/// `RECORD_MAX`.
pub(super) struct Listing<'a> {
    root: &'a SearchRoot,
    limit: usize,
    cap: usize,
    seen: usize,
    kept_bytes: usize,
    pending: Vec<u8>,
    kept: Vec<String>,
}

impl<'a> Listing<'a> {
    pub(super) fn new(root: &'a SearchRoot, limit: usize, cap: usize) -> Self {
        assert!(cap > 0, "a listing keeps at least one byte");
        Self {
            root,
            limit,
            cap,
            seen: 0,
            kept_bytes: 0,
            pending: Vec::new(),
            kept: Vec::new(),
        }
    }

    /// Bytes fd printed so far, kept or not.
    pub(super) fn seen(&self) -> usize {
        self.seen
    }

    /// Takes the complete records of `chunk`; says why to stop, if it should.
    pub(super) fn feed(&mut self, chunk: &[u8]) -> Option<Stop> {
        self.seen = self.seen.saturating_add(chunk.len());
        self.pending.extend_from_slice(chunk);
        let mut start = 0;
        while let Some(end) = self.pending[start..].iter().position(|byte| *byte == 0) {
            let record = self.pending[start..start + end].to_vec();
            start += end + 1;
            if let Some(stop) = self.take(&record) {
                self.pending.drain(..start);
                return Some(stop);
            }
        }
        self.pending.drain(..start);
        match self.pending.len() > RECORD_MAX {
            true => Some(Stop::Capped),
            false => None,
        }
    }

    /// At EOF, a last record without its NUL is complete.
    pub(super) fn finish(&mut self) -> Option<Stop> {
        let record = std::mem::take(&mut self.pending);
        self.take(&record)
    }

    fn take(&mut self, record: &[u8]) -> Option<Stop> {
        let entry = match record {
            [] => return None,
            record => self.root.entry(record)?,
        };
        let cost = entry.len() + 1;
        match self.kept_bytes + cost <= self.cap {
            true => {
                self.kept_bytes += cost;
                self.kept.push(entry);
            }
            false => return Some(Stop::Capped),
        }
        assert!(self.kept_bytes <= self.cap, "listing byte cap");
        match self.kept.len() > self.limit {
            true => Some(Stop::Enough),
            false => None,
        }
    }

    /// The kept entries, the limit and the skipped VCS directory.
    pub(super) fn into_parts(self) -> (Vec<String>, usize, Option<String>) {
        (self.kept, self.limit, self.root.skipped_vcs_dir.clone())
    }
}

/// `root` as read and edit would take it from the workspace. '.' is dropped
/// by text; '..' is resolved on disk, since text cannot see symlinks.
pub(super) fn shown_prefix(root: &Path, workspace: &Path) -> String {
    let (root, base) = match (without_dots(root), without_dots(workspace)) {
        (Some(root), Some(base)) => (root, base),
        _ => (on_disk(root), on_disk(workspace)),
    };
    match root.strip_prefix(&base) {
        Ok(relative) if relative.as_os_str().is_empty() => String::new(),
        Ok(relative) => format!("{}/", relative.to_string_lossy()),
        Err(_) => format!("{}/", root.to_string_lossy().trim_end_matches('/')),
    }
}

/// The path with '.' components dropped; `None` when it has a '..'.
fn without_dots(path: &Path) -> Option<PathBuf> {
    path.components()
        .try_fold(PathBuf::new(), |mut kept, component| match component {
            Component::CurDir => Some(kept),
            Component::ParentDir => None,
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                kept.push(component);
                Some(kept)
            }
        })
}

/// The canonical path; the path as given when it cannot be resolved (fd
/// then reports the missing root itself).
fn on_disk(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Where the VCS metadata directly under `root`, which the search skips,
/// lives. A `.git` file (a worktree or submodule) names it in its
/// `gitdir:` line; only a target that is or lies under a VCS directory is
/// believed, else the `.git` file itself is named.
pub(super) fn skipped_vcs_dir(root: &Path, workspace: &Path) -> Option<String> {
    VCS_METADATA_DIRS.iter().find_map(|name| {
        let path = root.join(name);
        let metadata = std::fs::metadata(&path).ok()?;
        let location = match (metadata.is_dir(), metadata.is_file() && *name == ".git") {
            (true, _) => path,
            (false, true) => git_dir_named_by(&path).unwrap_or(path),
            (false, false) => return None,
        };
        match shown_prefix(&location, workspace).trim_end_matches('/') {
            "" => None,
            shown => Some(shown.to_owned()),
        }
    })
}

/// The directory a `.git` file points at: an existing directory that is,
/// or lies under, a VCS metadata directory. The file is opened without
/// blocking and checked on the handle, so a FIFO swapped in never stalls.
fn git_dir_named_by(file: &Path) -> Option<PathBuf> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let handle = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(file)
        .ok()?;
    match handle.metadata().ok()?.is_file() {
        true => {}
        false => return None,
    }
    let mut text = String::new();
    handle.take(4096).read_to_string(&mut text).ok()?;
    let named = text.lines().next()?.strip_prefix("gitdir:")?.trim();
    let directory = std::fs::canonicalize(file.parent()?.join(Path::new(named))).ok()?;
    let under_vcs = directory.components().any(|component| match component {
        Component::Normal(name) => name
            .to_str()
            .is_some_and(|name| VCS_METADATA_DIRS.contains(&name)),
        Component::Prefix(_) | Component::RootDir | Component::CurDir | Component::ParentDir => {
            false
        }
    });
    match (under_vcs, directory.is_dir()) {
        (true, true) => Some(directory),
        (true, false) | (false, _) => None,
    }
}
