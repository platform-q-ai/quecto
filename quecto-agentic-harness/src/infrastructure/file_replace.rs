//! Replacing a file's contents whole or not at all (#2243): the `edit` and
//! `write` tools write through here, so a failed write (disk full, a quota,
//! an I/O error, the process killed) never leaves a file empty or cut short.
//!
//! The new contents go to a temporary file in the target's own directory —
//! hidden, named unpredictably (its name part capped so a long name still
//! fits) and created exclusively without following a link — which is given
//! the target's owner and exact mode, written, fsynced and renamed over the
//! target; the directory is then fsynced. On any error before the rename the
//! temporary file is removed (only while its name is still ours) and the
//! target is untouched.
//!
//! Where a rename cannot stand in for a plain write, the file is written in
//! place — from the start, cut to length, fsynced: not atomic, so a failure
//! part-way may leave it cut short or a mix of old and new text — and the
//! caller says so ([`Written::InPlace`]):
//! - **Hard links**: a rename would leave the other names on the old text.
//! - A **directory we cannot write**: no temporary file can be made there.
//! - A **rename the system refuses** (#2243 review): a file that is itself a
//!   mount point, such as a single-file bind mount (`EBUSY`, `EXDEV`), or
//!   another user's file in a sticky directory (`EPERM`).
//! - An **owner we cannot give back**: the temporary file is ours; another
//!   owner is given back (the whole owner, which only a privileged writer
//!   may; else the group alone, when it is one we are in), and when the
//!   result still differs the file is written in place instead, so it keeps
//!   its owner, group and mode. For our own file this is a no-op.
//!
//! Also:
//! - **Symbolic links** are kept: the link's final target is replaced, in
//!   its own directory. A link whose target does not exist is written
//!   through: its final target is created the same way, from a temporary
//!   file beside it, so a failed write leaves no partial target (#2254).
//! - A file we may not write is refused before anything is created, as a
//!   plain write would be: a rename must not replace what we could not
//!   write. The file is opened non-blocking and must be the regular file
//!   that was looked at (same device and inode): a FIFO or another file
//!   swapped in between is refused, not written or waited on.
//! - A rename makes a new inode: POSIX ACLs, extended attributes and
//!   SELinux labels of the old file are not carried over (a write in place
//!   keeps them).
//! - Races left: a file created at a missing name while its new contents
//!   are written is replaced by them; a swap of the file after it is opened
//!   and before the rename replaces the swapped-in file.

use std::fs::{File, Metadata, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::infrastructure::atomic_write::{sync_parent_dir, temp_path_in};

/// How the contents were written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Written {
    /// An existing file was replaced whole, by a rename.
    Replaced,
    /// There was no file: it was created.
    Created,
    /// The file was written in place, not atomically, and why.
    InPlace(InPlace),
}

/// Why a file was written in place. A `leftover` is a temporary file that
/// could not be removed (an append-only directory refuses the unlink).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InPlace {
    /// It has this many hard links, which a rename would break.
    HardLinks(u64),
    /// Its directory takes no new file, so no temporary file could be made.
    DirectoryNotWritable,
    /// The system refused to rename over it: a mount point, another user's
    /// file in a sticky directory, or a security policy (SELinux, AppArmor).
    RenameRefused { leftover: Option<PathBuf> },
    /// Its owner or group could not be given to a new file.
    OwnerNotKept { leftover: Option<PathBuf> },
}

/// A write that failed, and what it left.
#[derive(Debug)]
pub struct WriteFailure {
    pub error: io::Error,
    pub file: FileState,
    /// A temporary file that could not be removed (the directory refused
    /// the unlink): named so it can be deleted.
    pub leftover: Option<PathBuf>,
}

/// What a failed write left of the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileState {
    /// Nothing was changed: the file holds its earlier contents.
    Unchanged,
    /// There was no file, and none was created.
    Absent,
    /// The write stopped on an internal fault: what it left is not known.
    Unknown,
    /// It was being written in place: it may now be cut short, or a mix of
    /// old and new text.
    MayBeCutShort,
}

impl WriteFailure {
    fn unchanged(error: io::Error) -> Self {
        Self {
            error,
            file: FileState::Unchanged,
            leftover: None,
        }
    }

    fn absent(error: io::Error) -> Self {
        Self {
            error,
            file: FileState::Absent,
            leftover: None,
        }
    }

    fn with_leftover(self, leftover: Option<PathBuf>) -> Self {
        Self { leftover, ..self }
    }
}

/// A file's owner and group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Owner {
    uid: u32,
    gid: u32,
}

impl Owner {
    fn of(meta: &Metadata) -> Self {
        Self {
            uid: meta.uid(),
            gid: meta.gid(),
        }
    }
}

type Chown = fn(&File, Option<u32>, Option<u32>) -> io::Result<()>;

/// A step run once the temporary file exists, before it is written.
type AfterTemp<'a> = Box<dyn FnMut(&Path) -> io::Result<()> + 'a>;

/// The steps of a replacement that tests stand in for.
struct Steps<'a> {
    /// Runs once the temporary file exists, before it is written.
    after_temp: AfterTemp<'a>,
    rename: fn(&Path, &Path) -> io::Result<()>,
    chown: Chown,
    /// The owner the replaced file has.
    owner_of: fn(&Metadata) -> Owner,
    remove: fn(&Path) -> io::Result<()>,
    /// How many names the opened file has.
    links_of: fn(&Metadata) -> u64,
}

impl Steps<'static> {
    fn real() -> Self {
        Self {
            after_temp: Box::new(|_| Ok(())),
            rename: |from, to| std::fs::rename(from, to),
            chown: |file, uid, gid| std::os::unix::fs::fchown(file, uid, gid),
            owner_of: Owner::of,
            remove: |path| std::fs::remove_file(path),
            links_of: MetadataExt::nlink,
        }
    }
}

/// Replace the contents of the file at `path` (following links) with
/// `bytes`, or create it: see the module documentation.
pub fn replace_contents(path: &Path, bytes: &[u8]) -> Result<Written, WriteFailure> {
    replace_contents_with(path, bytes, Steps::real())
}

fn replace_contents_with(
    path: &Path,
    bytes: &[u8],
    steps: Steps<'_>,
) -> Result<Written, WriteFailure> {
    match Target::of(path).map_err(WriteFailure::unchanged)? {
        Target::Missing => {
            create_new(path, bytes, steps)?;
            Ok(Written::Created)
        }
        Target::DanglingLink => {
            // Created whole at the link's final target, beside it, so the
            // links are kept and a failed write leaves nothing (#2254).
            let target = dangling_target(path).map_err(WriteFailure::absent)?;
            create_new(&target, bytes, steps)?;
            Ok(Written::Created)
        }
        Target::File { path: real, meta } => replace_file(&real, &meta, bytes, steps),
    }
}

/// What `path` names.
enum Target {
    /// Nothing.
    Missing,
    /// A symbolic link (or chain) to nothing.
    DanglingLink,
    /// A regular file: its real path (links resolved) and its metadata.
    File { path: PathBuf, meta: Metadata },
}

impl Target {
    fn of(path: &Path) -> io::Result<Self> {
        let is_link = match std::fs::symlink_metadata(path) {
            Ok(meta) => meta.file_type().is_symlink(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::Missing),
            Err(error) => return Err(error),
        };
        let meta = match (std::fs::metadata(path), is_link) {
            (Ok(meta), _) => meta,
            (Err(error), true) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Self::DanglingLink);
            }
            (Err(error), _) => return Err(error),
        };
        match (meta.is_file(), meta.is_dir()) {
            (true, _) => {}
            (false, true) => return Err(io::Error::from(io::ErrorKind::IsADirectory)),
            (false, false) => return Err(not_a_regular_file()),
        }
        let path = match is_link {
            true => std::fs::canonicalize(path)?,
            false => path.to_path_buf(),
        };
        Ok(Self::File { path, meta })
    }
}

/// The path a chain of symbolic links from `path` ends at, which names
/// nothing: each relative target resolved against its link's directory.
/// An error when the chain is too long, or its end exists after all (made
/// meanwhile: it is not written over).
fn dangling_target(path: &Path) -> io::Result<PathBuf> {
    let mut link = path.to_path_buf();
    for _ in 0..40 {
        let target = std::fs::read_link(&link)?;
        let target = match link.parent() {
            Some(dir) if target.is_relative() => dir.join(target),
            Some(_) | None => target,
        };
        match std::fs::symlink_metadata(&target) {
            Ok(meta) if meta.file_type().is_symlink() => link = target,
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "the link's target was created meanwhile",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                // Its directory resolved, so the temporary file and the
                // rename name one plain directory.
                return match (target.parent(), target.file_name()) {
                    (Some(dir), Some(name)) => Ok(std::fs::canonicalize(dir)?.join(name)),
                    _ => Ok(target),
                };
            }
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::from_raw_os_error(libc::ELOOP))
}

fn not_a_regular_file() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "not a regular file")
}

/// Create `path` from a temporary file with a new file's default mode. On
/// failure no file is created; a temporary file that could not be removed
/// is named.
fn create_new(path: &Path, bytes: &[u8], mut steps: Steps<'_>) -> Result<(), WriteFailure> {
    let temp = new_temp(path, 0o666).map_err(WriteFailure::absent)?;
    let staged = fill(&temp, bytes, &mut steps).and_then(|()| (steps.rename)(&temp.path, path));
    match staged {
        Ok(()) => {
            sync_dir_after_rename(path);
            Ok(())
        }
        Err(error) => {
            let leftover = temp.remove(steps.remove);
            Err(WriteFailure::absent(error).with_leftover(leftover))
        }
    }
}

/// Open the file `meta` describes for writing, not truncated and without
/// blocking: refused exactly when a plain write would be, and refused when
/// what is open is not that regular file (swapped since it was looked at).
fn open_existing(path: &Path, meta: &Metadata) -> io::Result<File> {
    let file = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    let opened = file.metadata()?;
    let is_same = opened.dev() == meta.dev() && opened.ino() == meta.ino();
    match (opened.is_file(), is_same) {
        (true, true) => Ok(file),
        (false, _) => Err(not_a_regular_file()),
        (true, false) => Err(io::Error::other(
            "the file changed while it was being written; nothing was written",
        )),
    }
}

/// Replace an existing regular file: by a rename, or in place where a
/// rename cannot stand in for a plain write (see the module documentation).
fn replace_file(
    path: &Path,
    meta: &Metadata,
    bytes: &[u8],
    mut steps: Steps<'_>,
) -> Result<Written, WriteFailure> {
    let file = open_existing(path, meta).map_err(WriteFailure::unchanged)?;
    // The handle's own metadata: the file that will be written.
    let opened = file.metadata().map_err(WriteFailure::unchanged)?;
    // External state, never asserted: no name left means the file was
    // removed since it was opened (or a file system that reports none).
    match (steps.links_of)(&opened) {
        0 => {
            return Err(WriteFailure::unchanged(io::Error::new(
                io::ErrorKind::NotFound,
                "the file was removed while it was being written",
            )));
        }
        1 => {}
        links => {
            write_in_place(&file, bytes)?;
            return Ok(Written::InPlace(InPlace::HardLinks(links)));
        }
    }
    let mode = opened.mode() & 0o7777;
    let temp = match new_temp(path, mode) {
        Ok(created) => created,
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            write_in_place(&file, bytes)?;
            return Ok(Written::InPlace(InPlace::DirectoryNotWritable));
        }
        Err(error) => return Err(WriteFailure::unchanged(error)),
    };
    let want = (steps.owner_of)(&opened);
    match stage(path, &temp, bytes, want, mode, &mut steps) {
        Ok(None) => {
            sync_dir_after_rename(path);
            Ok(Written::Replaced)
        }
        Ok(Some(why)) => {
            let leftover = temp.remove(steps.remove);
            write_in_place(&file, bytes)?;
            Ok(Written::InPlace(why.with_leftover(leftover)))
        }
        Err(error) => {
            let leftover = temp.remove(steps.remove);
            Err(WriteFailure::unchanged(error).with_leftover(leftover))
        }
    }
}

/// Give the temporary file the owner and mode, fill it and rename it over
/// `path`: `None` once renamed, or why the file must be written in place.
fn stage(
    path: &Path,
    temp: &Temp,
    bytes: &[u8],
    want: Owner,
    mode: u32,
    steps: &mut Steps<'_>,
) -> io::Result<Option<InPlace>> {
    if !keep_owner(&temp.file, want, steps.chown)? {
        return Ok(Some(InPlace::OwnerNotKept { leftover: None }));
    }
    // The exact mode, whatever the umask. After the chown, which may clear
    // set-id bits; as with a plain write, the kernel may clear them again
    // when an unprivileged writer writes the file.
    temp.file
        .set_permissions(std::fs::Permissions::from_mode(mode))?;
    fill(temp, bytes, steps)?;
    match (steps.rename)(&temp.path, path) {
        Ok(()) => Ok(None),
        Err(error) if is_rename_refused(&error) => {
            Ok(Some(InPlace::RenameRefused { leftover: None }))
        }
        Err(error) => Err(error),
    }
}

/// A rename the system refuses where a plain write works: the target is a
/// mount point (`EBUSY`, `EXDEV`), another user's file in a sticky
/// directory, or a security policy (SELinux, AppArmor) or an append-only
/// directory refuses it (`EPERM`, `EACCES`; the directory takes new files:
/// the temporary file was made in it).
fn is_rename_refused(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ResourceBusy
            | io::ErrorKind::CrossesDevices
            | io::ErrorKind::PermissionDenied
    )
}

/// Give `file` the owner `want`: the whole owner (only a privileged writer
/// may), else the group alone (one we are in). Whether `file` now has it.
fn keep_owner(file: &File, want: Owner, chown: Chown) -> io::Result<bool> {
    let have = Owner::of(&file.metadata()?);
    if have == want {
        return Ok(true);
    }
    // A refused chown is not an error: the check below decides.
    if have.uid != want.uid {
        let _ = chown(file, Some(want.uid), Some(want.gid));
    }
    let have = Owner::of(&file.metadata()?);
    if have.gid != want.gid {
        let _ = chown(file, None, Some(want.gid));
    }
    Ok(Owner::of(&file.metadata()?) == want)
}

/// A temporary file: its name, and the handle it was created with.
struct Temp {
    path: PathBuf,
    file: File,
}

impl Temp {
    /// Remove it, only while its name is still ours: its path when it is
    /// still there (the unlink was refused), for the caller to name.
    fn remove(&self, remove: fn(&Path) -> io::Result<()>) -> Option<PathBuf> {
        let named = std::fs::symlink_metadata(&self.path).ok()?;
        let ours = self.file.metadata().ok()?;
        let is_ours = named.dev() == ours.dev() && named.ino() == ours.ino();
        match is_ours && remove(&self.path).is_err() {
            true => Some(self.path.clone()),
            false => None,
        }
    }
}

impl InPlace {
    /// This cause, naming the temporary file left behind, if any.
    fn with_leftover(self, left: Option<PathBuf>) -> Self {
        match self {
            Self::RenameRefused { .. } => Self::RenameRefused { leftover: left },
            Self::OwnerNotKept { .. } => Self::OwnerNotKept { leftover: left },
            Self::HardLinks(_) | Self::DirectoryNotWritable => self,
        }
    }
}

/// A new temporary file beside `path`, created with `mode`.
fn new_temp(path: &Path, mode: u32) -> io::Result<Temp> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the path names no file in a directory",
        ));
    };
    let temp = temp_path_in(dir, name);
    let file = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW)
        .mode(mode)
        .open(&temp)?;
    Ok(Temp { path: temp, file })
}

/// Write `bytes` to the temporary file and fsync it.
fn fill(temp: &Temp, bytes: &[u8], steps: &mut Steps<'_>) -> io::Result<()> {
    (steps.after_temp)(&temp.path)?;
    let mut writer = &temp.file;
    writer.write_all(bytes)?;
    temp.file.sync_all()
}

/// The rename has landed: the new contents are what `path` holds, so a
/// failed directory sync weakens only durability against a crash.
fn sync_dir_after_rename(path: &Path) {
    if let Err(error) = sync_parent_dir(path) {
        tracing::warn!(%error, path = %path.display(), "file_replace: directory sync failed after the rename");
    }
}

/// Write `bytes` over `file` in place: from the start, then cut to their
/// length, then fsync. Not atomic: a failure part-way may leave the file
/// cut short or a mix of old and new text, and says so.
fn write_in_place(file: &File, bytes: &[u8]) -> Result<(), WriteFailure> {
    let cut_short = |error| WriteFailure {
        error,
        file: FileState::MayBeCutShort,
        leftover: None,
    };
    let length = u64::try_from(bytes.len()).map_err(io::Error::other);
    let length = length.map_err(WriteFailure::unchanged)?;
    file.write_all_at(bytes, 0)
        .and_then(|()| file.set_len(length))
        .and_then(|()| file.sync_all())
        .map_err(cut_short)
}

#[cfg(test)]
#[path = "file_replace_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "file_replace_fallback_tests.rs"]
mod fallback_tests;
