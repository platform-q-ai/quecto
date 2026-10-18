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
