//! Creating a file this process may exec at once, without ever holding a
//! writable descriptor on it (#2232).
//!
//! Linux refuses to exec an inode that anything holds open for writing
//! (`ETXTBSY`). `std` opens files `O_CLOEXEC`, but a child forked by another
//! thread while our write descriptor is open keeps its copy until *its* own
//! `execve`; if we exec the file first, we get "Text file busy". Closing,
//! syncing or renaming here cannot help: the copy lives in someone else's
//! child and names the same inode.
//!
//! On Linux, therefore, this process creates the file with one exclusive,
//! no-follow open that is read-only (a read-only descriptor never makes an
//! exec busy) and hands that descriptor to a short-lived `cat` child as its
//! stdout. The child reopens it for writing through `/dev/stdout`, which on
//! Linux reopens the same inode, never the name: a symbolic link or FIFO
//! swapped into the name meanwhile is neither written through nor opened.
//!
//! Elsewhere (macOS: its `/dev/stdout` duplicates the read-only descriptor
//! rather than reopening the inode, so the child could not write) the bytes
//! are written in-process through a writable descriptor from the same
//! exclusive, no-follow open. Refusing to exec a file open for writing
//! (`ETXTBSY`) is a Linux (and some BSDs') behaviour; macOS does not enforce
//! it, so the in-process write there costs nothing.
//!
//! Either way the bytes are read back through our own descriptor before the
//! file is handed out: a writer that "succeeded" into somewhere else (a
//! missing `/dev/stdout` recreated as a plain file) is an error, never a
//! silently empty script.

use std::fs::File;
use std::io::Write;
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::process::{Command, Stdio};

/// The mode the new file has while it is written: owner-only, and owner
/// writable whatever the umask, so the child's reopen is never refused.
const WRITING_MODE: u32 = 0o600;

/// The search path the writer's `cat` is found on: the usual FHS places,
/// NixOS's system profile (`/run/current-system/sw/bin`) and Guix's
/// (`/run/current-system/profile/bin`); neither has a `/bin/cat`.
const WRITER_PATH: &str =
    "/usr/bin:/bin:/run/current-system/sw/bin:/run/current-system/profile/bin";

/// How the bytes reach the file.
#[derive(Debug, Clone, Copy)]
enum Writer {
    /// A `cat` child writes to a reopen of this path, which must name the
    /// child's stdout (our read-only descriptor): `/dev/stdout` on Linux.
    Child { reopen: &'static str },
    /// This process writes through a writable descriptor.
    InProcess,
}

/// The platform's writer; both are compiled (and tested) everywhere.
const PLATFORM_WRITER: Writer = if cfg!(target_os = "linux") {
    Writer::Child {
        reopen: "/dev/stdout",
    }
} else {
    Writer::InProcess
};

/// Create `path`, which must not exist, holding `contents` with mode 0600,
/// and return a handle on it. On Linux the handle is read-only and the only
/// writable descriptor ever on the file is held by a child that has exited
/// before this returns.
///
/// `AlreadyExists` when anything (a symbolic link included) has the name;
/// nothing is touched then. On any other failure the file this call created
/// is removed again, best effort: only while the name still holds it, and a
/// removal that fails is not reported over the write's own error.
pub fn create_new(path: &Path, contents: &[u8]) -> std::io::Result<File> {
    create_new_with(path, contents, PLATFORM_WRITER, || {})
}

/// [`create_new`] through `writer`, running `between` after the file is
/// created and before its bytes are written: the seam the name-swap tests
/// reach through.
fn create_new_with(
    path: &Path,
    contents: &[u8],
    writer: Writer,
    between: impl FnOnce(),
) -> std::io::Result<File> {
    assert!(
        path.file_name().is_some() && path.parent().is_some(),
        "{} names a file in a directory",
        path.display()
    );
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(matches!(writer, Writer::InProcess))
        .custom_flags(libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW)
        .mode(WRITING_MODE)
        .open(path)
        .map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("cannot create {}: {error}", path.display()),
            )
        })?;
    // `fchmod` ignores the umask the create was subject to (0277 would
    // leave 0400, and the writer's reopen refused).
    let written = file
        .set_permissions(std::fs::Permissions::from_mode(WRITING_MODE))
        .map_err(|error| format!("cannot set the mode: {error}"))
        .and_then(|()| {
            between();
            write_bytes(&file, contents, writer)
        })
        .and_then(|()| read_back(&file, contents));
    match written {
        Ok(()) => Ok(file),
        Err(error) => {
            remove_if_still_ours(path, &file);
            Err(std::io::Error::other(format!(
                "writing {}: {error}",
                path.display()
            )))
        }
    }
}

fn write_bytes(file: &File, contents: &[u8], writer: Writer) -> Result<(), String> {
    match writer {
        Writer::Child { reopen } => write_through_child(file, contents, reopen),
        // Positioned: the handle's cursor stays at 0 for its reader.
        Writer::InProcess => file
            .write_all_at(contents, 0)
            .map_err(|error| format!("cannot write: {error}")),
    }
}

/// Copy `contents` into `file` through a `cat` child whose stdout is `file`
/// and which writes to a reopen of `reopen` (its own stdout).
fn write_through_child(file: &File, contents: &[u8], reopen: &str) -> Result<(), String> {
    let mut child = Command::new("/bin/sh")
        .env_clear()
        .env("PATH", WRITER_PATH)
        .args(["-c", "exec cat > \"$1\"", "writer_free_file", reopen])
        .stdin(Stdio::piped())
        .stdout(Stdio::from(file.try_clone().map_err(|error| {
            format!("cannot hand the file to the writer: {error}")
        })?))
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot start the writer /bin/sh: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "the writer's stdin was not piped".to_string())?;
    // A writer that died early closes the pipe: its own status and stderr
    // say why, the feed error only adds context.
    let fed = stdin.write_all(contents);
    drop(stdin);
    let output = child
        .wait_with_output()
        .map_err(|error| format!("cannot wait for the writer: {error}"))?;
    match (output.status.success(), fed) {
        (true, Ok(())) => Ok(()),
        (_, fed) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let hint = if stderr.contains("Permission denied") {
                format!(
                    " (the writer reopens the new file through {reopen}; a umask or a {reopen} that is not the file itself can refuse that)"
                )
            } else {
                String::new()
            };
            Err(format!(
                "the writer failed ({}){}: {}{hint}",
                output.status,
                fed.err()
                    .map(|error| format!(" after feeding it failed: {error}"))
                    .unwrap_or_default(),
                stderr.trim()
            ))
        }
    }
}

/// The file holds exactly `contents`, read through our own descriptor.
fn read_back(file: &File, contents: &[u8]) -> Result<(), String> {
    let length = file
        .metadata()
        .map_err(|error| format!("cannot read back: {error}"))?
        .len();
    if length != contents.len() as u64 {
        return Err(format!(
            "the file holds {length} bytes, not the {} written: the writer wrote elsewhere",
            contents.len()
        ));
    }
    let mut held = vec![0; contents.len()];
    file.read_exact_at(&mut held, 0)
        .map_err(|error| format!("cannot read back: {error}"))?;
    if held == contents {
        Ok(())
    } else {
        Err("the file holds other bytes than those written".to_string())
    }
}

/// Remove `path` only while it still names `file`'s inode: never a file or
/// link someone else put in its place. Best effort: a failure is ignored.
fn remove_if_still_ours(path: &Path, file: &File) {
    let (Ok(named), Ok(ours)) = (std::fs::symlink_metadata(path), file.metadata()) else {
        return;
    };
    if named.dev() == ours.dev() && named.ino() == ours.ino() {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
#[path = "writer_free_file_tests.rs"]
mod tests;
