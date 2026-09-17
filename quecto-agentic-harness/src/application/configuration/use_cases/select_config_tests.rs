use super::*;
use std::path::PathBuf;

fn request(explicit: Option<&str>, cwd: Option<&str>) -> ConfigSelectionRequest {
    ConfigSelectionRequest {
        explicit: explicit.map(PathBuf::from),
        working_directory: cwd.map(PathBuf::from),
        global: PathBuf::from("/home/u/.quecto/config.json"),
    }
}

#[test]
fn an_explicit_override_replaces_both_layers() {
    let selected = SelectConfig::new().execute(request(Some("/x/c.json"), Some("/work")));
    assert_eq!(
        selected,
        ConfigSelection::Explicit(PathBuf::from("/x/c.json"))
    );
}

#[test]
fn the_working_directory_names_the_overlay_and_the_retired_local_file() {
    let selected = SelectConfig::new().execute(request(None, Some("/work")));
    assert_eq!(
        selected,
        ConfigSelection::Layered(ConfigLayers {
            global: PathBuf::from("/home/u/.quecto/config.json"),
            overlay: Some(PathBuf::from("/work/.quecto/config.json")),
            legacy_local: Some(PathBuf::from("/work/config.json")),
        })
    );
}

#[test]
fn without_a_working_directory_only_the_global_file_is_selected() {
    let selected = SelectConfig::new().execute(request(None, None));
    assert_eq!(
        selected,
        ConfigSelection::Layered(ConfigLayers {
            global: PathBuf::from("/home/u/.quecto/config.json"),
            overlay: None,
            legacy_local: None,
        })
    );
}
