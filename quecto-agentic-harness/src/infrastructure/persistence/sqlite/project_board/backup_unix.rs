//! Linux fd-relative filesystem primitives. No process-global cwd changes.
use super::{AdmittedBackupFile, BackupFileIdentity};
use std::{
    ffi::CString,
    fs::{File, Metadata, OpenOptions},
    io,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
};
fn unsafe_path() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "backup identity/path refused")
}
fn name(value: &str) -> io::Result<CString> {
    if matches!(
        Path::new(value).components().collect::<Vec<_>>().as_slice(),
        [std::path::Component::Normal(_)]
    ) {
        CString::new(value).map_err(|_| unsafe_path())
    } else {
        Err(unsafe_path())
    }
}
pub(in super::super) fn open(
    directory: &File,
    value: &str,
    flags: i32,
    mode: u32,
) -> io::Result<File> {
    let value = name(value)?;
    // SAFETY: the borrowed directory fd stays open; value is a live NUL-terminated CString.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            value.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode,
        )
    };
    if fd >= 0 {
        // SAFETY: successful openat returned a fresh owned fd, transferred exactly once to File.
        Ok(unsafe { File::from_raw_fd(fd) })
    } else {
        Err(io::Error::last_os_error())
    }
}
pub(in super::super) fn identity(meta: &Metadata) -> io::Result<BackupFileIdentity> {
    if meta.is_file() && meta.nlink() == 1 {
        Ok(BackupFileIdentity {
            device: meta.dev(),
            inode: meta.ino(),
            links: meta.nlink(),
        })
    } else {
        Err(unsafe_path())
    }
}
pub(in super::super) fn same(meta: &Metadata, expected: &BackupFileIdentity) -> bool {
    meta.is_file()
        && meta.dev() == expected.device
        && meta.ino() == expected.inode
        && meta.nlink() == expected.links
}
pub(in super::super) fn check(directory: &File, file: &AdmittedBackupFile) -> io::Result<()> {
    let handle = open(directory, &file.name, libc::O_RDONLY | libc::O_NONBLOCK, 0)?;
    if same(&handle.metadata()?, &file.identity) {
        Ok(())
    } else {
        Err(unsafe_path())
    }
}
pub(in super::super) fn remove(directory: &File, file: &AdmittedBackupFile) -> io::Result<()> {
    check(directory, file)?;
    let file = name(&file.name)?;
    // SAFETY: the held directory fd and validated single-component CString remain live for unlinkat.
    let result = unsafe { libc::unlinkat(directory.as_raw_fd(), file.as_ptr(), 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
pub(in super::super) fn link(directory: &File, from: &str, to: &str) -> io::Result<()> {
    let from = name(from)?;
    let to = name(to)?;
    // SAFETY: both CStrings and the borrowed directory fd remain live throughout linkat.
    let result = unsafe {
        libc::linkat(
            directory.as_raw_fd(),
            from.as_ptr(),
            directory.as_raw_fd(),
            to.as_ptr(),
            0,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
pub(in super::super) fn path(directory: &File, name: &str) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd())).join(name)
}
pub(in super::super) fn directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
        .open(path)
}
pub(in super::super) fn private_destination(
    to: &Path,
    source: &Path,
) -> io::Result<(File, PathBuf)> {
    let absolute = if to.is_absolute() {
        to.to_owned()
    } else {
        std::env::current_dir()?.join(to)
    };
    let leaf = absolute
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(unsafe_path)?;
    let parent = absolute.parent().ok_or_else(unsafe_path)?.canonicalize()?;
    // Existing parent aliases are intentionally admitted once, then every canonical component is anchored.
    let mut handle = directory(Path::new("/"))?;
    for component in parent.components().skip(1) {
        let text = component.as_os_str().to_str().ok_or_else(unsafe_path)?;
        handle = open(&handle, text, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
        let meta = handle.metadata()?;
        // SAFETY: geteuid takes no pointers and has no caller preconditions.
        let uid = unsafe { libc::geteuid() };
        if (meta.uid() == 0 || meta.uid() == uid)
            && (meta.mode() & 0o022 == 0 || (meta.uid() == 0 && meta.mode() & libc::S_ISVTX != 0))
        {
        } else {
            return Err(unsafe_path());
        }
    }
    let canonical = parent.join(leaf);
    let source = source.canonicalize()?;
    if canonical.strip_prefix(&source).is_err() && source.strip_prefix(&canonical).is_err() {
    } else {
        return Err(unsafe_path());
    }
    let leaf_c = name(leaf)?;
    // SAFETY: the held parent fd and validated leaf CString remain live throughout mkdirat.
    let made = unsafe { libc::mkdirat(handle.as_raw_fd(), leaf_c.as_ptr(), 0o700) };
    if made == 0 || io::Error::last_os_error().kind() == io::ErrorKind::AlreadyExists {
    } else {
        return Err(io::Error::last_os_error());
    }
    let destination = open(&handle, leaf, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
    let meta = destination.metadata()?;
    // SAFETY: geteuid takes no pointers and has no caller preconditions.
    if meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o777 == 0o700 {
    } else {
        return Err(unsafe_path());
    }
    durable_destination(&handle, destination, canonical)
}
/// Sync the held parent even on an existing-leaf retry: a previous uncertain
/// mkdir must not become a falsely durable success on the next invocation.
pub(in super::super) fn durable_destination(
    parent: &File,
    destination: File,
    canonical: PathBuf,
) -> io::Result<(File, PathBuf)> {
    parent.sync_all()?;
    Ok((destination, canonical))
}
pub(in super::super) fn stable_directory(file: &File, path: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    let held = file.metadata()?;
    if meta.is_dir() && meta.dev() == held.dev() && meta.ino() == held.ino() {
        Ok(())
    } else {
        Err(unsafe_path())
    }
}
/// The board's backup lock: held only while `.{board}.backup.lock` still
/// names the very inode this run flocked.
pub(in super::super) fn lock(directory: &File, board: &str) -> io::Result<File> {
    let handle = open_lock(directory, board)?;
    #[cfg(test)]
    between_open_and_flock::run();
    hold(directory, board, handle)
}
pub(in super::super) fn open_lock(directory: &File, board: &str) -> io::Result<File> {
    let handle = open(
        directory,
        &lock_name(board),
        libc::O_RDWR | libc::O_CREAT | libc::O_NONBLOCK,
        0o600,
    )?;
    let meta = handle.metadata()?;
    if meta.is_file()
        && meta.nlink() == 1
        // SAFETY: geteuid takes no pointers and has no caller preconditions.
        && meta.uid() == unsafe { libc::geteuid() }
        && meta.mode() & 0o777 == 0o600
    {
        Ok(handle)
    } else {
        Err(unsafe_path())
    }
}
/// Flocks `handle`, then asks the held directory (no-follow) whether the lock
/// name still names that inode. A name renamed or unlinked and recreated
/// since the open refuses: this run would hold an inode no other run locks.
/// Dropping the refused handle releases its flock.
pub(in super::super) fn hold(directory: &File, board: &str, handle: File) -> io::Result<File> {
    // SAFETY: handle owns a live fd for flock; its File keeps the acquired lock alive on success.
    let result = unsafe { libc::flock(handle.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
    } else {
        return Err(io::Error::last_os_error());
    }
    let held = handle.metadata()?;
    let named = match open(directory, &lock_name(board), libc::O_PATH, 0) {
        Ok(named) => named.metadata()?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(unsafe_path()),
        Err(error) => return Err(error),
    };
    if named.is_file() && named.dev() == held.dev() && named.ino() == held.ino() {
        Ok(handle)
    } else {
        Err(unsafe_path())
    }
}
pub(in super::super) fn lock_name(board: &str) -> String {
    format!(".{board}.backup.lock")
}
/// Test seam: lets a test swap the lock name inside `lock()`'s window
/// between its open and its flock. Thread-local, so tests stay independent.
#[cfg(test)]
pub(in super::super) mod between_open_and_flock {
    use std::cell::RefCell;
    thread_local! {
        static HOOK: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    }
    /// Runs `hook` once, on this thread's next `lock()`.
    pub(in super::super::super) fn set(hook: impl FnOnce() + 'static) {
        HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
    }
    pub(super) fn run() {
        if let Some(hook) = HOOK.with(|slot| slot.borrow_mut().take()) {
            hook();
        }
    }
}
