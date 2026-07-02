use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// Atomically replace `path` with `bytes` by writing a same-directory temporary
/// file and renaming it into place.
pub fn atomic_write(path: &Path, bytes: &[u8], mode: Option<u32>) -> io::Result<()> {
    let tmp_path = temp_path_for(path)?;
    let write_result = write_temp_file(&tmp_path, bytes, mode)
        .and_then(|()| std::fs::rename(&tmp_path, path))
        .and_then(|()| sync_parent_dir(path));

    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }

    write_result
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
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn atomic_write_replaces_existing_file_without_truncating_first() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("data.json");
        std::fs::write(&target, b"old contents").unwrap();

        atomic_write(&target, b"new contents", None).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new contents");
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_applies_requested_unix_mode() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("secret.json");

        atomic_write(&target, b"secret", Some(0o600)).unwrap();

        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn temp_path_accepts_non_utf8_file_names() {
        use std::os::unix::ffi::OsStringExt;

        let tmp = TempDir::new().unwrap();
        let name = OsString::from_vec(vec![b'c', b'r', b'e', b'd', 0xff]);
        let target = tmp.path().join(name);

        let temp = temp_path_for(&target).unwrap();

        assert_eq!(temp.parent(), Some(tmp.path()));
    }
}
