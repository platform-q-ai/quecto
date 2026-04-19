//! Contract tests for the `OnboardStore` port.

use quecto::domain::workspace::OnboardStore;
use quecto::infrastructure::persistence::workspace_store::FileOnboardStore;
use std::sync::Arc;

fn under_test(base_dir: std::path::PathBuf) -> Arc<dyn OnboardStore> {
    Arc::new(FileOnboardStore::new(base_dir))
}

#[test]
fn paths_are_rooted_at_base_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path().to_path_buf());
    assert!(store.config_path().starts_with(tmp.path()));
    assert!(store.workspace_path().starts_with(tmp.path()));
}

#[test]
fn config_exists_is_false_before_initialize_and_true_after() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path().to_path_buf());

    assert!(!store.config_exists().unwrap(),
        "config must not exist before initialize");

    store.initialize(&[("config.toml", "key = \"value\"\n")]).unwrap();

    assert!(store.config_exists().unwrap(),
        "config must exist after initialize writes it");
}

#[test]
fn initialize_writes_all_supplied_templates_verbatim() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path().to_path_buf());
    store.initialize(&[
        ("config.toml", "key = \"value\"\n"),
        ("README.md", "# hi\n"),
    ]).unwrap();

    let cfg = std::fs::read_to_string(store.workspace_path().join("config.toml")).unwrap();
    assert_eq!(cfg, "key = \"value\"\n");
    let readme = std::fs::read_to_string(store.workspace_path().join("README.md")).unwrap();
    assert_eq!(readme, "# hi\n");
}
