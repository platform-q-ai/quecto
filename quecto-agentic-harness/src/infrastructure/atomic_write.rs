use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// Atomically replace `path` with `bytes` by writing a same-directory temporary
/// file and renaming it into place.
///
/// A failure of the trailing directory-metadata sync (see `sync_parent_dir`)
/// does not fail the call: by that point `rename` has already succeeded, so
/// `bytes` is the file any reader will see. Only unlink the temp file on a
/// failure that happens *before* the rename — once renamed, there is no temp
/// file left to clean up.
pub fn atomic_write(path: &Path, bytes: &[u8], mode: Option<u32>) -> io::Result<()> {
    let tmp_path = temp_path_for(path)?;

    if let Err(e) = write_temp_file(&tmp_path, bytes, mode) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    // The rename has landed; `bytes` is now what `path` contains regardless of
    // whether the directory-metadata sync below succeeds, so a sync failure
    // (e.g. an unusual filesystem or a transient EIO on the directory) must
    // not be reported as a failed write — the caller already has the new
    // content. This only weakens the durability guarantee against a
    // concurrent crash, not the correctness of any subsequent read.
    if let Err(e) = sync_parent_dir(path) {
        tracing::warn!(error = %e, path = %path.display(), "atomic_write: parent directory sync failed after a successful rename");
    }
    Ok(())
}

fn temp_path_for(path: &Path) -> io::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "atomic write path has no parent",
        )
    })?;
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "atomic write path has no file name",
        )
    })?;

    let mut tmp_name = OsString::from(".");
    tmp_name.push(file_name);
    tmp_name.push(".");
    tmp_name.push(uuid::Uuid::new_v4().to_string());
    tmp_name.push(".tmp");
    Ok(parent.join(tmp_name))
}

#[cfg(unix)]
fn sync_parent_dir(path: &Path) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "atomic write path has no parent",
        )
    })?;
    std::fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_dir(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn write_temp_file(path: &Path, bytes: &[u8], mode: Option<u32>) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::fs::PermissionsExt;

    let mode = mode.unwrap_or(0o666);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn write_temp_file(path: &Path, bytes: &[u8], _mode: Option<u32>) -> io::Result<()> {
    use std::io::Write;

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
#[path = "atomic_write_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
#[path = "atomic_write_tests.rs"]
mod tests;
