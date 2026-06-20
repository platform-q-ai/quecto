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

    assert!(
        !store.config_exists().unwrap(),
        "config must not exist before initialize"
    );

    store.initialize().unwrap();

    assert!(
        store.config_exists().unwrap(),
        "config must exist after initialize writes it"
    );
}

#[test]
fn initialize_creates_config_and_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let store = under_test(tmp.path().to_path_buf());
    store.initialize().unwrap();

    assert!(store.config_path().exists(), "config file must be created");
    assert!(
        store.workspace_path().is_dir(),
        "workspace dir must be created"
    );
}
