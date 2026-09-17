use super::*;

#[test]
fn canonical_directory_required() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        FilesystemScope.canonicalize(dir.path()).unwrap(),
        dir.path().canonicalize().unwrap()
    );
    assert!(
        FilesystemScope
            .canonicalize(&dir.path().join("missing"))
            .is_err()
    );
    let file = dir.path().join("file");
    std::fs::write(&file, "").unwrap();
    assert!(FilesystemScope.canonicalize(&file).is_err());
}
