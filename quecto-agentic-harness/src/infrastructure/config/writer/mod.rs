//! The one configuration writer (#2024): a JSON document laid down
//! minimal-diff (the file's own indentation kept, two spaces for a new
//! file, key order and unknown keys as the document carries them, a
//! trailing newline) and atomically (tmp + fsync + rename through
//! `atomic_write`). It serves the configuration capability's
//! [`ConfigDocumentWriter`] port and the tool-policy persistence hook
//! (`tool_policy`), which patches only `tools.policy.entries`.
//!
//! A rename keeps a file whole but not an *update*: two patchers that read
//! the same content both rename their own result in, and one's key is
//! gone. Every read → patch → validate → write cycle therefore runs under
//! an exclusive `flock(2)` ([`exclusive_hold`]), which blocks across
//! processes and threads alike. The lock file lives under the base
//! directory (`<base_dir>/locks/<sha256 of the document's canonical
//! path>.lock`), never beside the document: taking the hold on a
//! repository's not-yet-written `.quecto/config.json` must not create
//! `.quecto/` there, and a refused write leaves a clean checkout clean.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::application::configuration::ports::{ConfigDocumentWriter, DocumentLock};
use crate::infrastructure::atomic_write::atomic_write;

pub mod tool_policy;

/// The mode a configuration file is created with: it may hold API keys.
const NEW_CONFIG_MODE: u32 = 0o600;

/// The directory under the base directory that holds every document lock.
pub const LOCK_DIR_NAME: &str = "locks";

/// Where the locks of the documents written under `base_dir` live.
pub fn lock_dir_for(base_dir: &Path) -> PathBuf {
    base_dir.join(LOCK_DIR_NAME)
}

/// The writer bound to one base directory: every hold it hands out is a
/// file under [`lock_dir_for`] of that directory.
#[derive(Debug, Clone)]
pub struct JsonDocumentWriter {
    lock_dir: PathBuf,
}

impl JsonDocumentWriter {
    pub fn for_base_dir(base_dir: &Path) -> Self {
        Self {
            lock_dir: lock_dir_for(base_dir),
        }
    }
}

impl ConfigDocumentWriter for JsonDocumentWriter {
    fn write(&self, path: &Path, document: &serde_json::Value) -> Result<Vec<u8>, String> {
        write_document(path, document)
    }

    fn exclusive(&self, path: &Path) -> Result<Box<dyn DocumentLock>, String> {
        exclusive_hold(&self.lock_dir, path).map(|hold| Box::new(hold) as Box<dyn DocumentLock>)
    }
}

/// The lock file the exclusive hold on `path` is taken on:
/// `<lock_dir>/<sha256 hex of the document's canonical path>.lock`. The
/// canonical path resolves every link on the way (a symlinked global
/// file and its target share one lock) and, for a document that does not
/// exist yet, its deepest existing ancestor plus the remaining names —
/// so two creators of the same file contend for the same lock.
pub fn lock_path(lock_dir: &Path, path: &Path) -> Result<PathBuf, String> {
    let identity = canonical_identity(path)?;
    let digest = Sha256::digest(identity.as_os_str().as_encoded_bytes());
    Ok(lock_dir.join(format!("{digest:x}.lock")))
}

/// `path` with every existing prefix canonicalised: the file itself when
/// it exists, else its deepest existing ancestor with the missing tail
/// appended as named. A path with no existing ancestor is an error.
fn canonical_identity(path: &Path) -> Result<PathBuf, String> {
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut probe = path;
    loop {
        match std::fs::canonicalize(probe) {
            Ok(canonical) => {
                let mut identity = canonical;
                for name in tail.iter().rev() {
                    identity.push(name);
                }
                return Ok(identity);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = probe
                    .file_name()
                    .ok_or_else(|| format!("{} has no existing ancestor", path.display()))?;
                tail.push(name.to_os_string());
                probe = probe
                    .parent()
                    .ok_or_else(|| format!("{} has no existing ancestor", path.display()))?;
            }
            Err(error) => {
                return Err(format!("cannot resolve {}: {error}", probe.display()));
            }
        }
    }
}

/// An exclusive `flock` on the document's lock file, released on drop
/// (explicitly, so a duplicate of the descriptor a forked child still
/// holds does not keep the lock alive).
#[derive(Debug)]
pub struct ExclusiveHold {
    file: std::fs::File,
}

impl DocumentLock for ExclusiveHold {}

impl Drop for ExclusiveHold {
    #[expect(clippy::incompatible_msrv)]
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Take the exclusive hold on `path`'s lock file under `lock_dir`,
/// blocking until any other holder — in this process or another —
/// releases it. Only `lock_dir` is created; the document's own directory
/// is not touched (that is `write_document`'s, after validation). The
/// lock file is created 0600 like the document it guards (a
/// world-writable lock would let any co-resident user wedge every
/// configuration write).
//
// `File::lock` stabilized in 1.89; the crate's tests already call it, so
// 1.89 is the real toolchain floor — clippy.toml's declared 1.85 predates
// it and awaits a coordinated MSRV bump.
#[expect(clippy::incompatible_msrv)]
pub fn exclusive_hold(lock_dir: &Path, path: &Path) -> Result<ExclusiveHold, String> {
    let lock_path = lock_path(lock_dir, path)?;
    std::fs::create_dir_all(lock_dir).map_err(|error| {
        format!(
            "failed to create {} for the config lock: {error}",
            lock_dir.display()
        )
    })?;
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(NEW_CONFIG_MODE);
    }
    let file = options
        .open(&lock_path)
        .map_err(|error| format!("failed to open {}: {error}", lock_path.display()))?;
    file.lock()
        .map_err(|error| format!("failed to lock {}: {error}", lock_path.display()))?;
    Ok(ExclusiveHold { file })
}

/// Render `document` in the layout `path` already uses and replace the
/// file atomically, returning the bytes written. An existing file keeps
/// its mode; a new one is private to the user, and its directory is
/// created here — after the caller validated the document — not before.
pub fn write_document(path: &Path, document: &serde_json::Value) -> Result<Vec<u8>, String> {
    // A symlinked config (dotfiles) is written through the link: the rename
    // would otherwise replace the link with a plain file and leave the
    // linked copy stale.
    let target = resolve_symlink(path)?;
    let path = target.as_path();
    let existing = match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    let bytes = render(document, existing.as_deref());
    let mode = existing_mode(path).unwrap_or(NEW_CONFIG_MODE);
    atomic_write(path, &bytes, Some(mode)).map_err(|error| error.to_string())?;
    Ok(bytes)
}

/// The file a write lands in: the final target when `path` is a symlink
/// (a dangling link is an error naming it), else `path` itself.
fn resolve_symlink(path: &Path) -> Result<std::path::PathBuf, String> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            std::fs::canonicalize(path).map_err(|error| {
                format!(
                    "{} is a symlink that cannot be resolved: {error}",
                    path.display()
                )
            })
        }
        _ => Ok(path.to_path_buf()),
    }
}

/// Pretty-print with the indentation the existing file used (two spaces
/// for a new or compact file) and a trailing newline, so a file already in
/// that shape changes only on the lines whose values changed.
fn render(document: &serde_json::Value, existing: Option<&[u8]>) -> Vec<u8> {
    let indent = existing
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .and_then(detect_indent)
        .unwrap_or_else(|| "  ".to_string());
    let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
    let mut bytes = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut bytes, formatter);
    serde::Serialize::serialize(document, &mut serializer).expect("a JSON value serializes");
    bytes.push(b'\n');
    bytes
}

/// The leading whitespace of the first indented line, when the text has
/// one; a compact single-line document yields `None`.
fn detect_indent(text: &str) -> Option<String> {
    text.lines()
        .skip(1)
        .map(|line| &line[..line.len() - line.trim_start().len()])
        .find(|indent| !indent.is_empty())
        .map(str::to_string)
}

#[cfg(unix)]
fn existing_mode(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn existing_mode(_path: &Path) -> Option<u32> {
    None
}

#[cfg(test)]
#[path = "writer_tests.rs"]
mod tests;
