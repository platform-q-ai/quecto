use super::*;
use crate::application::configuration::dto::{ConfigSelection, ConfigSelectionRequest};
use tempfile::TempDir;

#[test]
fn composed_use_case_selects_a_real_local_file() {
    let cwd = TempDir::new().unwrap();
    let local = cwd.path().join("config.json");
    std::fs::write(&local, "{}").unwrap();
    let selected = build_select_config()
        .execute(ConfigSelectionRequest {
            explicit: None,
            working_directory: Some(cwd.path().to_path_buf()),
            global: "/nowhere/config.json".into(),
        })
        .unwrap();
    assert_eq!(selected, ConfigSelection::WorkingDirectory(local));
}
