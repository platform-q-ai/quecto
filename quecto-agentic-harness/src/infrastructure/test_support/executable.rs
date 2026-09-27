//! Race-free test executables (#2232).
//!
//! A test that writes a fake binary with `std::fs::write` and then runs it
//! fails with `ETXTBSY` ("Text file busy") whenever another thread of the test
//! process forks while the write descriptor is open: the forked child keeps
//! its copy until its own `execve`, and the kernel refuses to exec an inode
//! with any writer. [`write_executable`] writes through
//! [`writer_free_file::create_new`] (on Linux a `cat` child holds the only
//! writable descriptor), sets the mode through the handle it returns
//! (`fchmod`, no reopen by name) and renames into place, so an existing
//! executable (possibly running) is replaced, never truncated.
//!
//! The TUI, which does not depend on this crate, keeps its own copy of this
//! helper in `quecto-tui/src/shell/test_executable.rs`.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::infrastructure::processes::writer_free_file;

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
    let file =
        writer_free_file::create_new(&staging, contents).unwrap_or_else(|error| panic!("{error}"));
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

/// Run `work` while `forkers` threads spawn `/bin/true` back to back: the
/// load under which a descriptor held open in this process leaks into
/// another thread's child and makes an exec "Text file busy" (#2232). The
/// spawns take `std`'s `posix_spawn` path (a vfork: no page-table copy), so
/// the load stays light for tests running alongside, yet each child still
/// holds a copy of every descriptor until its exec.
pub fn while_forking<R>(forkers: usize, work: impl FnOnce() -> R) -> R {
    assert!(forkers > 0, "a stress run needs at least one forker");
    let forking = Forkers::start(forkers, fork_target());
    let result = work();
    let forks = forking.stop();
    assert!(
        forks > 0,
        "no fork succeeded while the work ran: the stress created no load"
    );
    result
}

/// The program the forkers spawn: the first `true` that runs, tried once.
/// None is a clear failure, never a loop that spins without forking.
fn fork_target() -> &'static str {
    ["/bin/true", "/usr/bin/true", "true"]
        .into_iter()
        .find(|program| {
            std::process::Command::new(program)
                .status()
                .is_ok_and(|status| status.success())
        })
        .expect("no `true` could be spawned (tried /bin/true, /usr/bin/true, PATH): the stress test cannot create fork load")
}

/// The forker threads of [`while_forking`]. Dropping them, on a panic in
/// the work too, stops and joins every one: none outlives its test.
struct Forkers {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    forks: std::sync::Arc<std::sync::atomic::AtomicU64>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Forkers {
    fn start(count: usize, target: &'static str) -> Self {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let forks = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        let threads = (0..count)
            .map(|_| {
                let (stop, forks) = (stop.clone(), forks.clone());
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        if std::process::Command::new(target).status().is_ok() {
                            forks.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                })
            })
            .collect();
        Self {
            stop,
            forks,
            threads,
        }
    }

    /// Stop and join the threads, reporting a forker that panicked; the
    /// number of spawns that succeeded.
    fn stop(mut self) -> u64 {
        self.stop.store(true, Ordering::Relaxed);
        for thread in std::mem::take(&mut self.threads) {
            thread.join().expect("a forker thread panicked");
        }
        self.forks.load(Ordering::Relaxed)
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
    match std::process::Command::new(path).output() {
        Ok(_) => false,
        Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) => true,
        Err(error) => panic!("exec {}: {error}", path.display()),
    }
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

#[cfg(test)]
#[path = "executable_tests.rs"]
mod tests;
