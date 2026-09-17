use super::*;
use crate::application::configuration::dto::{
    ConfigLayers, ConfigSelectionRequest, OverlayTrustRequest,
};
use std::path::PathBuf;
use tempfile::TempDir;

fn env(base_dir: &Path) -> ConfigurationEnvironment {
    ConfigurationEnvironment {
        base_dir: base_dir.to_path_buf(),
        prompt_for_trust: false,
    }
}

#[test]
fn the_composed_graph_selects_trusts_and_merges_a_real_overlay() {
    let base = TempDir::new().unwrap();
    let cwd = TempDir::new().unwrap();
    let global = base.path().join("config.json");
    std::fs::write(
        &global,
        r#"{"agents":{"defaults":{"model":"global","effort":"high"}}}"#,
    )
    .unwrap();
    let overlay = cwd.path().join(".quecto").join("config.json");
    std::fs::create_dir_all(overlay.parent().unwrap()).unwrap();
    std::fs::write(&overlay, r#"{"agents":{"defaults":{"model":"local"}}}"#).unwrap();

    let handles = build_configuration_handles(&env(base.path()));
    let selection = handles.select.execute(ConfigSelectionRequest {
        explicit: None,
        working_directory: Some(cwd.path().to_path_buf()),
        global: global.clone(),
    });
    assert_eq!(
        selection,
        ConfigSelection::Layered(ConfigLayers {
            global: global.clone(),
            overlay: Some(overlay.clone()),
            legacy_local: Some(cwd.path().join("config.json")),
        })
    );
    let untrusted = handles.resolve.execute(&selection).unwrap();
    assert_eq!(untrusted.document["agents"]["defaults"]["model"], "global");

    handles
        .trust
        .execute(OverlayTrustRequest {
            path: overlay.clone(),
        })
        .unwrap();
    assert!(base.path().join("config-overlay-trust.json").exists());
    let trusted = handles.resolve.execute(&selection).unwrap();
    assert_eq!(trusted.document["agents"]["defaults"]["model"], "local");
    assert_eq!(trusted.document["agents"]["defaults"]["effort"], "high");

    let loader = build_config_loader(base.path(), selection, HashMap::new());
    let config = loader().unwrap();
    assert_eq!(config.agents.defaults.model, "local");
    assert_eq!(config.agents.defaults.effort.as_deref(), Some("high"));
    assert!(format!("{handles:?}").contains("ConfigurationHandles"));
}

#[test]
fn the_loader_reports_a_broken_selected_file() {
    let base = TempDir::new().unwrap();
    let explicit = base.path().join("explicit.json");
    std::fs::write(&explicit, "{ nope").unwrap();
    let loader = build_config_loader(
        base.path(),
        ConfigSelection::Explicit(explicit.clone()),
        HashMap::new(),
    );
    let error = loader().unwrap_err();
    assert!(error.contains(&explicit.display().to_string()), "{error}");
}

#[test]
fn a_reload_watches_the_base_file_the_overlay_and_the_trust_record() {
    let base = Path::new("/home/u/.quecto");
    let layered = ConfigSelection::Layered(ConfigLayers {
        global: base.join("config.json"),
        overlay: Some(PathBuf::from("/work/.quecto/config.json")),
        legacy_local: None,
    });
    assert_eq!(
        watched_config_files(base, &layered),
        vec![
            base.join("config.json"),
            PathBuf::from("/work/.quecto/config.json"),
            base.join("config-overlay-trust.json"),
        ]
    );
    let explicit = ConfigSelection::Explicit(PathBuf::from("/x/c.json"));
    assert_eq!(
        watched_config_files(base, &explicit),
        vec![PathBuf::from("/x/c.json")]
    );
}
