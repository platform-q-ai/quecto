//! A follower's token file (epic #2705 B11): the device token it reaches
//! its primary with, read from a file and never from the command line. The
//! file is judged before it is opened (lstat: a regular file, never a
//! symlink, FIFO or device, so nothing can hang the server's start), then
//! opened without following a link and without blocking, and judged again
//! as opened: the same file, this user's alone, at most 4 KiB. No error
//! quotes what it holds.
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use super::state_dir::this_uid;

/// The most bytes a token file may hold.
pub const TOKEN_FILE_BYTES: u64 = 4096;

/// The token in the file at `path`, its line end trimmed.
///
/// # Errors
/// A file that is not this user's alone, not a regular file, past 4 KiB,
/// or not text, each said without quoting it.
pub fn read_token_file(path: &Path) -> Result<String, String> {
    let shown = path.display();
    let failed = |error: std::io::Error| format!("the token file {shown}: {error}");
    let private = |meta: &std::fs::Metadata| {
        meta.file_type().is_file()
            && meta.uid() == this_uid()
            && meta.mode() & 0o077 == 0
            && meta.len() <= TOKEN_FILE_BYTES
    };
    let refused = || {
        format!(
            "the token file {shown} must be a regular file of this user's, at most 4 KiB, \
             that no one else may read (mode 0600 or 0400)"
        )
    };
    let seen = std::fs::symlink_metadata(path).map_err(failed)?;
    if !private(&seen) {
        return Err(refused());
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(failed)?;
    let opened = file.metadata().map_err(failed)?;
    match (
        private(&opened),
        (opened.dev(), opened.ino()) == (seen.dev(), seen.ino()),
    ) {
        (true, true) => {}
        (_, _) => return Err(refused()),
    }
    let mut text = String::new();
    (file.take(TOKEN_FILE_BYTES).read_to_string(&mut text))
        .map_err(|_| format!("the token file {shown} is not one line of text"))?;
    let token = text.strip_suffix('\n').unwrap_or(&text);
    Ok(token.strip_suffix('\r').unwrap_or(token).to_string())
}

#[cfg(test)]
#[path = "token_file_tests.rs"]
mod tests;
