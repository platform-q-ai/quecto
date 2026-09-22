use super::parent_playbook::load;

const NAME: &str = "PARENT_PLAYBOOK.md";

#[test]
fn falls_back_to_packaged_playbook_only_when_absent() {
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(
        load(directory.path()).unwrap(),
        include_str!("../../../PARENT_PLAYBOOK.md")
    );
    std::fs::write(directory.path().join(NAME), "project policy\n").unwrap();
    assert_eq!(load(directory.path()).unwrap(), "project policy\n");
}

#[test]
fn rejects_invalid_and_unreadable_override() {
    let directory = tempfile::tempdir().unwrap();
    let override_path = directory.path().join(NAME);
    std::fs::write(&override_path, [0xff]).unwrap();
    assert!(load(directory.path()).unwrap_err().contains("UTF-8"));
    std::fs::remove_file(&override_path).unwrap();
    std::fs::create_dir(&override_path).unwrap();
    assert!(
        load(directory.path())
            .unwrap_err()
            .contains("failed to read")
    );
}

#[test]
fn never_searches_ancestors() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join(NAME), "ancestor").unwrap();
    let child = directory.path().join("child");
    std::fs::create_dir(&child).unwrap();
    assert_eq!(
        load(&child).unwrap(),
        include_str!("../../../PARENT_PLAYBOOK.md")
    );
}
