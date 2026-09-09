use super::*;
use tempfile::TempDir;

#[test]
fn canonical_aliases_resolve_to_one_identity() {
    let temp = TempDir::new().unwrap();
    let target = temp.path().join("target");
    std::fs::create_dir(&target).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, temp.path().join("alias")).unwrap();
    assert_eq!(
        resolve_native_folder(&target, None),
        resolve_native_folder(&temp.path().join("alias"), None)
    );
}

#[test]
fn non_directory_is_explicitly_unavailable() {
    let temp = TempDir::new().unwrap();
    let file = temp.path().join("file");
    std::fs::write(&file, b"x").unwrap();
    assert_eq!(
        resolve_native_folder(&file, None),
        CurrentFolderScope::Unavailable(FolderScopeUnavailableReason::NotADirectory)
    );
}
