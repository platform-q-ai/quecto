//! Atomic file replace for durable TUI sidecars (#1465 AC4).
//!
//! Writes go to a same-directory temporary file, then `rename` replaces the
//! destination so readers never observe a partial authoritative file.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

/// Atomically write `bytes` to `path` via temp file + rename.
///
/// The temporary name is `.<file_name>.tmp-<pid>-<nonce>` in the same directory
/// so rename stays on one filesystem.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_name = format!(".{file_name}.tmp-{}-{nonce}", std::process::id());
    let tmp_path = parent.join(tmp_name);
    let result = (|| {
        let mut f = File::create(&tmp_path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp_path, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

#[cfg(test)]
#[path = "atomic_file_tests.rs"]
mod atomic_file_tests;
