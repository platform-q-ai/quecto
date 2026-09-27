//! Race-free test executables (#2232): the TUI's copy of the harness's
//! `quecto-agentic-harness/src/infrastructure/test_support/executable.rs`
//! and of the writer it builds on,
//! `quecto-agentic-harness/src/infrastructure/processes/writer_free_file.rs`.
//! The TUI does not depend on the harness, and the only crate both share,
//! `quecto-line-io`, is the wire-framing library; a crate for this one
//! helper is not worth a workspace member. Keep the copies in step.
//!
//! A test that writes a fake binary with `std::fs::write` and then runs it
//! fails with `ETXTBSY` ("Text file busy") whenever another thread of the test
//! process forks while the write descriptor is open: `O_CLOEXEC` closes the
//! child's copy only at *its* `execve`, and the kernel refuses to exec an
//! inode with any writer. Closing, syncing or renaming here cannot help.
//!
//! So this process never holds a writable descriptor on the executable: it
//! creates the file with one exclusive, no-follow, read-only open, a `cat`
//! child writes the bytes through `/dev/stdout` (a reopen of that same
//! inode, never of the name: a Linux behaviour, and CI runs these tests
//! on Linux), the mode is set through the read-only handle, and the file
//! is renamed into place, so an existing executable (possibly running) is
//! replaced, never truncated.

use std::fs::File;
use std::io::Write;
use std::os::unix::fs::{FileExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// The mode every test executable is given: owner rwx, others r-x.
pub const EXECUTABLE_MODE: u32 = 0o755;

/// Write `contents` to `path` as an executable ([`EXECUTABLE_MODE`]) that can
/// be exec'd immediately, from any thread, with no `ETXTBSY` race.
///
/// Panics (it is test support) when the file cannot be written; the panic
/// names the path and the writer's own error.
pub fn write_executable(path: &Path, contents: impl AsRef<[u8]>) {
    let contents = contents.as_ref();
    let staging = staging_path(path);
    let file = create_through_child(&staging, contents);
    file.set_permissions(std::fs::Permissions::from_mode(EXECUTABLE_MODE))
        .unwrap_or_else(|error| panic!("chmod {}: {error}", staging.display()));
    drop(file);
    std::fs::rename(&staging, path).unwrap_or_else(|error| {
        panic!(
            "rename {} -> {}: {error}",
            staging.display(),
            path.display()
        )
    });
    let written =
        std::fs::read(path).unwrap_or_else(|error| panic!("read back {}: {error}", path.display()));
    assert_eq!(
        written,
        contents,
        "{} holds the bytes written",
        path.display()
    );
}

/// A sibling of `path` no other call can pick: same directory (so the rename
/// is atomic), unique per process and per call.
fn staging_path(path: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .unwrap_or_else(|| panic!("{} names a file", path.display()));
    let mut staging = name.to_os_string();
    staging.push(format!(
        ".{}.{}.staging",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    path.with_file_name(staging)
}

/// Create `path` read-only and exclusively, then have a `/bin/cat` child
/// write `contents` into a reopen of its inode; return the read-only handle.
fn create_through_child(path: &Path, contents: &[u8]) -> File {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW)
        .mode(0o600)
        .open(path)
        .unwrap_or_else(|error| panic!("cannot create {}: {error}", path.display()));
    // `fchmod` ignores the umask the create was subject to, so the writer's
    // reopen is never refused (umask 0277 would leave 0400).
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .unwrap_or_else(|error| panic!("chmod {}: {error}", path.display()));
    // `cat` from a fixed search path: NixOS's (`/run/current-system/sw/bin`)
    // and Guix's (`/run/current-system/profile/bin`) system profiles, which
    // have no `/bin/cat`, after the usual places.
    let mut child = Command::new("/bin/sh")
        .env_clear()
        .env(
            "PATH",
            "/usr/bin:/bin:/run/current-system/sw/bin:/run/current-system/profile/bin",
        )
        .args(["-c", "exec cat > /dev/stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::from(file.try_clone().expect("clone the handle")))
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("cannot start the writer /bin/sh: {error}"));
    let mut stdin = child.stdin.take().expect("the writer's stdin is piped");
    let fed = stdin.write_all(contents);
    drop(stdin);
    let output = child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("wait for the writer of {}: {error}", path.display()));
    assert!(
        output.status.success() && fed.is_ok(),
        "writing {} failed ({}, feed: {fed:?}): {}",
        path.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    // Read back through our own descriptor: a writer that "succeeded" into
    // somewhere else (a missing `/dev/stdout` recreated as a plain file)
    // must not leave an empty script behind.
    let mut held = vec![0; contents.len()];
    let length = file.metadata().expect("stat the new file").len();
    assert!(
        length == contents.len() as u64
            && file.read_exact_at(&mut held, 0).is_ok()
            && held == contents,
        "{} holds {length} bytes, not the {} written: the writer wrote elsewhere",
        path.display(),
        contents.len()
    );
    file
}

/// Run `work` while `forkers` threads spawn `/bin/true` back to back: the
/// load under which a descriptor held open in this process leaks into
/// another thread's child (#2232). `posix_spawn` keeps the load light, yet
/// each child holds every descriptor until its exec.
pub fn while_forking<R>(forkers: usize, work: impl FnOnce() -> R) -> R {
    assert!(forkers > 0, "a stress run needs at least one forker");
    let forking = Forkers::start(forkers);
    let result = work();
    forking.stop();
    result
}

/// The forker threads of [`while_forking`]. Dropping them, on a panic in
/// the work too, stops and joins every one: none outlives its test.
struct Forkers {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Forkers {
    fn start(count: usize) -> Self {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let threads = (0..count)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        let _ = Command::new("/bin/true").status();
                    }
                })
            })
            .collect();
        Self { stop, threads }
    }

    /// Stop and join the threads, reporting a forker that panicked.
    fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in std::mem::take(&mut self.threads) {
            thread.join().expect("a forker thread panicked");
        }
    }
}

impl Drop for Forkers {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in std::mem::take(&mut self.threads) {
            let _ = thread.join();
        }
    }
}

/// Whether running `path` failed with `ETXTBSY`; any other failure panics
/// with the path, and a run is otherwise waited for.
pub fn exec_is_busy(path: &Path) -> bool {
    match Command::new(path).output() {
        Ok(_) => false,
        Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) => true,
        Err(error) => panic!("exec {}: {error}", path.display()),
    }
}

#[cfg(test)]
#[path = "test_executable_tests.rs"]
mod tests;
