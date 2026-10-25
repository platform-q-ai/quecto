//! A board server's state directory (epic #2705 B5, #2741): the one
//! allowlist `serve`, `import` and `export` reach it through, and the
//! exclusive lock one server (or one import) holds on it.
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::Path;

/// What a second server is told when a server holds the directory.
pub const SERVER_HOLDS: &str =
    "another board server holds it: one server per state directory (give this one --state <dir>)";

fn refused(dir: &Path, why: &str) -> String {
    format!("the state directory {}: {why}", dir.display())
}

/// `dir` is absolute and does not end in `/` (which would follow a
/// symlink): all a read needs.
/// # Errors
/// Either is not so.
pub fn absolute_dir(dir: &Path) -> Result<(), String> {
    match (
        dir.is_absolute(),
        dir.as_os_str().as_bytes().ends_with(b"/"),
    ) {
        (true, false) => Ok(()),
        (false, _) => Err(refused(dir, "not absolute")),
        (true, true) => Err(refused(dir, "must not end in '/'")),
    }
}

/// `dir` as an absolute directory of mode 0700 of this user's, made so if
/// missing; a symlink is never followed.
/// # Errors
/// Not absolute, ending in `/`, not made, or not this user's at 0700.
pub fn private_dir(dir: &Path) -> Result<(), String> {
    absolute_dir(dir)?;
    match fs::symlink_metadata(dir) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let made = fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir);
            made.map_err(|error| refused(dir, &error.to_string()))?;
        }
        Err(error) => return Err(refused(dir, &error.to_string())),
    }
    let meta = fs::symlink_metadata(dir).map_err(|error| refused(dir, &error.to_string()))?;
    let mode = meta.permissions().mode() & 0o777;
    match meta.is_dir() && meta.uid() == this_uid() && mode == 0o700 {
        true => Ok(()),
        false => Err(refused(
            dir,
            "must be a directory of this user's, mode 0700 (a symlink is not followed)",
        )),
    }
}

/// The state directory itself, opened never through a symlink and locked
/// exclusively without waiting: no file removal undoes it. `held` is the
/// refusal while another holds it.
/// # Errors
/// Not opened, held by another, or not locked.
// `File::try_lock` stabilized in 1.89 (as the admission `SingletonLock` notes).
#[expect(clippy::incompatible_msrv)]
pub fn locked(state: &Path, held: &str) -> Result<StateDirLock, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let dir = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(state)
        .map_err(|error| refused(state, &error.to_string()))?;
    match dir.try_lock() {
        Ok(()) => Ok(StateDirLock(dir)),
        Err(fs::TryLockError::WouldBlock) => Err(refused(state, held)),
        Err(fs::TryLockError::Error(error)) => Err(refused(state, &error.to_string())),
    }
}

/// The locked state directory, unlocked when dropped: explicitly, since a
/// `flock` belongs to the open file description, which a concurrent fork
/// shares until its child runs its program (as `SessionOwnershipGuard`, #1492).
#[derive(Debug)]
pub struct StateDirLock(fs::File);

impl std::ops::Deref for StateDirLock {
    type Target = fs::File;
    fn deref(&self) -> &fs::File {
        &self.0
    }
}

impl Drop for StateDirLock {
    // `File::unlock` stabilized with `try_lock` in 1.89.
    #[expect(clippy::incompatible_msrv)]
    fn drop(&mut self) {
        // An unlock that fails leaves the lock to the descriptor's close.
        let _ = self.0.unlock();
    }
}

/// This process's effective user id.
pub(crate) fn this_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}
