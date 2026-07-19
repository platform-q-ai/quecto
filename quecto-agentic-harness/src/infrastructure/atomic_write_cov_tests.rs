use super::*;
use tempfile::TempDir;

#[test]
fn temp_path_creates_parent_and_uses_hidden_same_dir_name() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("nested").join("data.json");

    let temp = temp_path_for(&target).unwrap();

    assert!(target.parent().unwrap().is_dir());
    assert_eq!(temp.parent(), target.parent());
    let name = temp.file_name().unwrap().to_string_lossy();
    assert!(name.starts_with(".data.json."), "{name}");
    assert!(name.ends_with(".tmp"), "{name}");
}

#[test]
fn temp_path_rejects_path_without_file_name() {
    let err = temp_path_for(Path::new("/")).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
}

#[test]
fn sync_parent_dir_reports_missing_parent() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("missing").join("file.txt");
    let err = sync_parent_dir(&target).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::NotFound);
}
