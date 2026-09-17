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

/// A run from the base directory's parent (typically `$HOME`) or from the
/// base directory itself must not see the global file as its own overlay
/// or as a retired local file.
#[test]
fn the_global_file_is_never_its_own_overlay_or_legacy_candidate() {
    let selected = SelectConfig::new().execute(request(None, Some("/home/u")));
    assert_eq!(
        selected,
        ConfigSelection::Layered(ConfigLayers {
            global: PathBuf::from("/home/u/.quecto/config.json"),
            overlay: None,
            legacy_local: Some(PathBuf::from("/home/u/config.json")),
        })
    );
    let selected = SelectConfig::new().execute(request(None, Some("/home/u/.quecto")));
    assert_eq!(
        selected,
        ConfigSelection::Layered(ConfigLayers {
            global: PathBuf::from("/home/u/.quecto/config.json"),
            overlay: Some(PathBuf::from("/home/u/.quecto/.quecto/config.json")),
            legacy_local: None,
        })
    );
    let selected = SelectConfig::new().execute(request(None, Some("/home/u/src/../")));
    assert_eq!(selected.overlay_path(), None, "normalised before comparing");
}
