use super::*;

#[test]
fn into_path_unwraps_every_variant() {
    let path = PathBuf::from("/x/config.json");
    for selection in [
        ConfigSelection::Explicit(path.clone()),
        ConfigSelection::WorkingDirectory(path.clone()),
        ConfigSelection::Global(path.clone()),
    ] {
        assert_eq!(selection.into_path(), path);
    }
}

#[test]
fn only_the_global_selection_may_be_absent() {
    let path = PathBuf::from("/x/config.json");
    assert!(ConfigSelection::Explicit(path.clone()).must_exist());
    assert!(ConfigSelection::WorkingDirectory(path.clone()).must_exist());
    assert!(!ConfigSelection::Global(path.clone()).must_exist());
    assert_eq!(ConfigSelection::Global(path.clone()).path(), path.as_path());
}

#[test]
fn error_display_names_the_path_and_the_remedy() {
    let path = PathBuf::from("/work/config.json");
    let not_regular = ConfigSelectionError {
        path: path.clone(),
        rejection: LocalConfigRejection::NotRegularFile,
    }
    .to_string();
    assert!(not_regular.contains("/work/config.json"), "{not_regular}");
    assert!(not_regular.contains("not a regular file"), "{not_regular}");
    assert!(not_regular.contains("--config"), "{not_regular}");

    let unreadable = ConfigSelectionError {
        path,
        rejection: LocalConfigRejection::Unreadable("permission denied".into()),
    }
    .to_string();
    assert!(
        unreadable.contains("cannot be read: permission denied"),
        "{unreadable}"
    );
    assert!(unreadable.contains("--config"), "{unreadable}");
}
