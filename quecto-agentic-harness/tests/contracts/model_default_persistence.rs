//! Contract for the `ModelDefaultPersistence` port (#2024 S2): a record
//! lands `agents.defaults.model` in exactly the layer the scope names,
//! reports that file, leaves every other key of the file and the other
//! layer untouched, records trust for an overlay, and refuses — writing
//! nothing — when the overlay's current content is not trusted.
use std::sync::Arc;
use tempfile::TempDir;

use quecto::application::catalogue::ports::{DefaultScope, ModelDefaultPersistence};
use quecto::application::configuration::dto::{ConfigLayers, ConfigSelection};
use quecto::composition::configuration::build_configuration_handles;
use quecto::infrastructure::config::writer::defaults::ConfigDefaultsWriter;
use quecto::interface::cli::configuration_handles::ConfigurationEnvironment;

pub(crate) struct Layers {
    pub base: TempDir,
    pub cwd: TempDir,
    pub global: std::path::PathBuf,
    pub overlay: std::path::PathBuf,
}

pub(crate) fn layers(global_content: &str) -> Layers {
    let base = TempDir::new().unwrap();
    let cwd = TempDir::new().unwrap();
    let global = base.path().join("config.json");
    std::fs::write(&global, global_content).unwrap();
    let overlay = cwd.path().join(".quecto").join("config.json");
    Layers {
        base,
        cwd,
        global,
        overlay,
    }
}

pub(crate) fn writer(layers: &Layers) -> ConfigDefaultsWriter {
    let handles = build_configuration_handles(&ConfigurationEnvironment {
        base_dir: layers.base.path().to_path_buf(),
        prompt_for_trust: false,
    });
    ConfigDefaultsWriter::new(
        handles.patch,
        ConfigSelection::Layered(ConfigLayers {
            global: layers.global.clone(),
            overlay: Some(layers.overlay.clone()),
            legacy_local: Some(layers.cwd.path().join("config.json")),
        }),
    )
}

pub(crate) fn json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn under_test(layers: &Layers) -> Arc<dyn ModelDefaultPersistence> {
    Arc::new(writer(layers))
}

#[test]
fn a_local_record_lands_in_the_overlay_only_and_is_trusted() {
    let fx = layers(r#"{"agents":{"defaults":{"model":"g/m"}},"providers":{"p":{"api_key":"k"}}}"#);
    let before = std::fs::read(&fx.global).unwrap();
    let persisted = under_test(&fx)
        .persist_model(DefaultScope::Local, "acme/m")
        .unwrap();
    assert_eq!(persisted.scope, DefaultScope::Local);
    assert_eq!(persisted.path, fx.overlay);
    assert_eq!(json(&fx.overlay)["agents"]["defaults"]["model"], "acme/m");
    assert_eq!(std::fs::read(&fx.global).unwrap(), before);
    let trust = json(&fx.base.path().join("config-overlay-trust.json"));
    assert!(
        trust["approved"]
            .as_object()
            .is_some_and(|approved| !approved.is_empty()),
        "{trust}"
    );
}

#[test]
fn a_global_record_lands_in_the_global_file_only_and_keeps_every_other_key() {
    let fx = layers(
        r#"{"custom":"kept","agents":{"defaults":{"model":"g/m","effort":"high"}},"providers":{"p":{"api_key":"k"}}}"#,
    );
    let persisted = under_test(&fx)
        .persist_model(DefaultScope::Global, "acme/m")
        .unwrap();
    assert_eq!(persisted.scope, DefaultScope::Global);
    assert_eq!(persisted.path, fx.global);
    let document = json(&fx.global);
    assert_eq!(document["agents"]["defaults"]["model"], "acme/m");
    assert_eq!(document["agents"]["defaults"]["effort"], "high");
    assert_eq!(document["custom"], "kept");
    assert_eq!(document["providers"]["p"]["api_key"], "k");
    assert!(!fx.overlay.exists());
}

#[test]
fn an_untrusted_overlay_is_refused_and_left_byte_identical() {
    let fx = layers("{}");
    std::fs::create_dir_all(fx.overlay.parent().unwrap()).unwrap();
    std::fs::write(&fx.overlay, r#"{"agents":{"defaults":{"model":"theirs"}}}"#).unwrap();
    let before = std::fs::read(&fx.overlay).unwrap();
    let error = under_test(&fx)
        .persist_model(DefaultScope::Local, "acme/m")
        .unwrap_err();
    assert!(error.contains("not trusted"), "{error}");
    assert_eq!(std::fs::read(&fx.overlay).unwrap(), before);
}
