use super::*;

fn layers(overlay: bool) -> ConfigLayers {
    ConfigLayers {
        global: PathBuf::from("/home/u/.quecto/config.json"),
        overlay: overlay.then(|| PathBuf::from("/work/.quecto/config.json")),
        legacy_local: overlay.then(|| PathBuf::from("/work/config.json")),
    }
}

#[test]
fn the_base_path_is_the_explicit_file_or_the_global_file() {
    let explicit = ConfigSelection::Explicit(PathBuf::from("/x/config.json"));
    assert_eq!(explicit.path(), Path::new("/x/config.json"));
    assert_eq!(
        explicit.clone().into_path(),
        PathBuf::from("/x/config.json")
    );
    assert_eq!(explicit.overlay_path(), None);

    let layered = ConfigSelection::Layered(layers(true));
    assert_eq!(layered.path(), Path::new("/home/u/.quecto/config.json"));
    assert_eq!(
        layered.overlay_path(),
        Some(Path::new("/work/.quecto/config.json"))
    );
    assert_eq!(
        layered.into_path(),
        PathBuf::from("/home/u/.quecto/config.json")
    );
}

#[test]
fn only_the_explicit_selection_must_exist() {
    assert!(ConfigSelection::Explicit(PathBuf::from("/x")).must_exist());
    assert!(!ConfigSelection::Layered(layers(true)).must_exist());
    assert!(!ConfigSelection::Layered(layers(false)).must_exist());
}

#[test]
fn a_layered_selection_without_a_working_directory_has_no_overlay() {
    let selection = ConfigSelection::Layered(layers(false));
    assert_eq!(selection.overlay_path(), None);
}
