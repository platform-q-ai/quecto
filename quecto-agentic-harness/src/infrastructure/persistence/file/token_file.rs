//! A server's small private files (epic #2705 B11): a follower's token file
//! (the device token it reaches its primary with, read from a file and
//! never from the command line) and its diverged mark. A file is judged
//! before it is opened (lstat: a regular file, never a symlink, FIFO or
//! device, so nothing can hang the server's start), then opened without
//! following a link and without blocking, and judged again as opened: the
//! same file, this user's alone, at most 4 KiB. No error quotes what a
//! token file holds.
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use super::state_dir::this_uid;

/// The most bytes a private file may hold.
pub const TOKEN_FILE_BYTES: u64 = 4096;

/// Whether `meta` is a private file's: regular, this user's, no group or
/// other bits, at most [`TOKEN_FILE_BYTES`].
fn private(meta: &std::fs::Metadata) -> bool {
    meta.file_type().is_file()
        && meta.uid() == this_uid()
        && meta.mode() & 0o077 == 0
        && meta.len() <= TOKEN_FILE_BYTES
}

/// What the private file at `path` holds; `None` while there is none.
///
/// # Errors
/// A file that is not private (`what` names it), or one not read.
pub fn read_private(path: &Path, what: &str) -> Result<Option<String>, String> {
    let shown = path.display();
    let failed = |error: std::io::Error| format!("{what} {shown}: {error}");
    let refused = || {
        format!(
            "{what} {shown} must be a regular file of this user's, at most 4 KiB, \
             that no one else may read (mode 0600 or 0400)"
        )
    };
    let seen = match std::fs::symlink_metadata(path) {
        Ok(seen) => seen,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failed(error)),
    };
    match private(&seen) {
        true => {}
        false => return Err(refused()),
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(failed)?;
    let opened = file.metadata().map_err(failed)?;
    let same = (opened.dev(), opened.ino()) == (seen.dev(), seen.ino());
    match (private(&opened), same) {
        (true, true) => {}
        (_, _) => return Err(refused()),
    }
    let mut text = String::new();
    (file.take(TOKEN_FILE_BYTES).read_to_string(&mut text))
        .map_err(|_| format!("{what} {shown} is not text"))?;
    Ok(Some(text))
}

/// Writes `text` as the private file at `path`, mode 0600: a file already
/// there must be a private one (never a link, FIFO or device).
///
/// # Errors
/// What is there is not a private file, or the write failed.
pub fn write_private(path: &Path, what: &str, text: &str) -> Result<(), String> {
    let shown = path.display();
    let failed = |error: std::io::Error| format!("{what} {shown}: {error}");
    match std::fs::symlink_metadata(path) {
        Ok(seen) if private(&seen) => {}
        Ok(_) => return Err(format!("{what} {shown} is not a private regular file")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(failed(error)),
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(failed)?;
    let bytes: Vec<u8> = text.bytes().take(TOKEN_FILE_BYTES as usize).collect();
    (file.write_all(&bytes))
        .and_then(|()| file.sync_all())
        .map_err(failed)
}

/// The token in the file at `path`, its line end trimmed.
///
/// # Errors
/// A file missing, not this user's alone, not a regular file, past 4 KiB,
/// or not text, each said without quoting it.
pub fn read_token_file(path: &Path) -> Result<String, String> {
    let what = "the token file";
    let missing = || format!("{what} {}: there is none", path.display());
    let text = read_private(path, what)?.ok_or_else(missing)?;
    let token = text.strip_suffix('\n').unwrap_or(&text);
    Ok(token.strip_suffix('\r').unwrap_or(token).to_string())
}

#[cfg(test)]
#[path = "token_file_tests.rs"]
mod tests;
