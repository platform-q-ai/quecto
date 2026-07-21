use super::*;

fn write_executable(path: &Path) {
    std::fs::write(path, b"#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).unwrap();
    }
}

#[test]
fn falls_back_from_deleted_current_exe_to_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let binary = dir.path().join("quecto");
    write_executable(&binary);

    let resolved = resolve_child_binary_with(
        || Ok(PathBuf::from("/home/me/.cargo/bin/quecto (deleted)")),
        vec![OsString::from("quecto")].into_iter(),
        || Some(dir.path().as_os_str().to_os_string()),
    )
    .unwrap();

    assert_eq!(resolved, binary);
}

#[test]
fn skips_stale_argv0_path_and_uses_path_fallback() {
    let dir = tempfile::TempDir::new().unwrap();
    let binary = dir.path().join("quecto");
    write_executable(&binary);

    let resolved = resolve_child_binary_with(
        || Ok(PathBuf::from("/home/me/.cargo/bin/quecto (deleted)")),
        vec![OsString::from("./target/release/quecto")].into_iter(),
        || Some(dir.path().as_os_str().to_os_string()),
    )
    .unwrap();

    assert_eq!(resolved, binary);
}

#[test]
fn skips_non_executable_path_candidate() {
    let non_executable_dir = tempfile::TempDir::new().unwrap();
    let executable_dir = tempfile::TempDir::new().unwrap();
    std::fs::write(non_executable_dir.path().join("quecto"), b"not executable").unwrap();
    let binary = executable_dir.path().join("quecto");
    write_executable(&binary);
    let path_var =
        std::env::join_paths([non_executable_dir.path(), executable_dir.path()]).unwrap();

    let resolved = resolve_child_binary_with(
        || Ok(PathBuf::from("/home/me/.cargo/bin/quecto (deleted)")),
        vec![OsString::from("quecto")].into_iter(),
        || Some(path_var.clone()),
    )
    .unwrap();

    assert_eq!(resolved, binary);
}

#[test]
fn returns_current_exe_when_not_deleted() {
    let expected = PathBuf::from("/tmp/quecto");
    let resolved = resolve_child_binary_with(
        || Ok(expected.clone()),
        vec![OsString::from("quecto")].into_iter(),
        || None,
    )
    .unwrap();
    assert_eq!(resolved, expected);
}
