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
fn a_reload_watches_the_trust_record_only_when_an_overlay_exists_at_seed_time() {
    let base = Path::new("/home/u/.quecto");
    let work = TempDir::new().unwrap();
    let overlay = work.path().join(".quecto").join("config.json");
    let layered = ConfigSelection::Layered(ConfigLayers {
        global: base.join("config.json"),
        overlay: Some(overlay.clone()),
        legacy_local: None,
    });
    let watched = |selection: &ConfigSelection| -> Vec<PathBuf> {
        watched_config_sources(base, selection)
            .iter()
            .map(|source| source.path().to_path_buf())
            .collect()
    };
    // No overlay: the overlay location is watched (its creation is a
    // change), the host-wide trust record is not (another repository's
    // `config trust` is no change for this session).
    assert_eq!(
        watched(&layered),
        vec![base.join("config.json"), overlay.clone()]
    );
    // An overlay that exists: its removal and its trust are both changes.
    std::fs::create_dir_all(overlay.parent().unwrap()).unwrap();
    std::fs::write(&overlay, "{}").unwrap();
    assert_eq!(
        watched(&layered),
        vec![
            base.join("config.json"),
            overlay,
            base.join("config-overlay-trust.json"),
        ]
    );
    let explicit = ConfigSelection::Explicit(PathBuf::from("/x/c.json"));
    assert_eq!(watched(&explicit), vec![PathBuf::from("/x/c.json")]);
}

#[test]
fn concurrent_patches_of_one_file_all_land() {
    const WRITERS: usize = 16;
    let base = TempDir::new().unwrap();
    let global = base.path().join("config.json");
    std::fs::write(&global, "{}").unwrap();
    let selection = ConfigSelection::Explicit(global.clone());
    let barrier = Arc::new(std::sync::Barrier::new(WRITERS));
    let workers: Vec<_> = (0..WRITERS)
        .map(|i| {
            let base_dir = base.path().to_path_buf();
            let selection = selection.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let handles = build_configuration_handles(&env(&base_dir));
                barrier.wait();
                handles
                    .patch
                    .execute(crate::application::configuration::dto::ConfigPatch {
                        selection,
                        layer: crate::application::configuration::dto::ConfigLayer::Global,
                        key_path: format!("k{i}"),
                        value: serde_json::json!(i),
                    })
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
    let document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&global).unwrap()).unwrap();
    let object = document.as_object().unwrap();
    assert_eq!(object.len(), WRITERS, "every patch landed: {document}");
    for i in 0..WRITERS {
        assert_eq!(object[&format!("k{i}")], serde_json::json!(i));
    }
    let leftovers: Vec<_> = std::fs::read_dir(base.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert!(
        global.with_extension("json.lock").exists(),
        "the sidecar lock"
    );
}
