//! The default-persistence mapping over a real composed patch handle: the
//! record lands in the layer the scope names with the S1 writer's
//! guarantees (trust recorded, other keys untouched, refusals named), and
//! the unbound writer refuses everything.

use super::super::configuration::build_configuration_handles;
use super::*;
use crate::application::configuration::dto::ConfigLayers;
use crate::application::configuration::ports::{OverlayTrust, OverlayTrustStore};
use crate::infrastructure::config::persistence::PersistentOverlayTrustStore;
use crate::interface::cli::configuration_handles::ConfigurationEnvironment;
use tempfile::TempDir;

struct Fixture {
    _base: TempDir,
    _cwd: TempDir,
    global: std::path::PathBuf,
    overlay: std::path::PathBuf,
    trust_record: std::path::PathBuf,
    writer: ConfigDefaultsWriter,
}

fn fixture(global_content: &str) -> Fixture {
    let base = TempDir::new().unwrap();
    let cwd = TempDir::new().unwrap();
    let global = base.path().join("config.json");
    std::fs::write(&global, global_content).unwrap();
    let overlay = cwd.path().join(".quecto").join("config.json");
    let handles = build_configuration_handles(&ConfigurationEnvironment {
        base_dir: base.path().to_path_buf(),
        prompt_for_trust: false,
    });
    let selection = ConfigSelection::Layered(ConfigLayers {
        global: global.clone(),
        overlay: Some(overlay.clone()),
        legacy_local: None,
    });
    Fixture {
        trust_record: base.path().join("config-overlay-trust.json"),
        _base: base,
        _cwd: cwd,
        global,
        overlay,
        writer: ConfigDefaultsWriter::new(handles.patch, selection),
    }
}

fn json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn a_local_record_creates_the_trusted_overlay_and_leaves_the_global_file_alone() {
    let fx = fixture(
        r#"{"agents":{"defaults":{"model":"g/m"}},"providers":{"openai":{"api_key":"sk"}}}"#,
    );
    let before = std::fs::read(&fx.global).unwrap();
    let persisted = fx
        .writer
        .persist_model(DefaultScope::Local, "acme/m")
        .unwrap();
    assert_eq!(persisted.scope, DefaultScope::Local);
    assert_eq!(persisted.path, fx.overlay);
    assert_eq!(json(&fx.overlay)["agents"]["defaults"]["model"], "acme/m");
    assert_eq!(std::fs::read(&fx.global).unwrap(), before);
    assert!(fx.trust_record.exists());
    let on_disk = std::fs::read(&fx.overlay).unwrap();
    let trust = PersistentOverlayTrustStore::for_base_dir(fx._base.path(), false);
    assert_eq!(
        trust.decide(&fx.overlay, &on_disk),
        OverlayTrust::Trusted,
        "trust is recorded for the bytes the writer laid down"
    );
}

#[test]
fn a_global_record_changes_only_the_addressed_key() {
    let fx = fixture(
        r#"{"unknown":"kept","agents":{"defaults":{"model":"g/m"}},"providers":{"openai":{"api_key":"sk"}}}"#,
    );
    let persisted = fx
        .writer
        .persist_effort(DefaultScope::Global, EffortLevel::High)
        .unwrap();
    assert_eq!(persisted.scope, DefaultScope::Global);
    assert_eq!(persisted.path, fx.global);
    let document = json(&fx.global);
    assert_eq!(document["agents"]["defaults"]["effort"], "high");
    assert_eq!(document["agents"]["defaults"]["model"], "g/m");
    assert_eq!(document["unknown"], "kept");
    assert_eq!(document["providers"]["openai"]["api_key"], "sk");
    assert!(!fx.overlay.exists());
}

#[test]
fn an_untrusted_overlay_is_refused_with_the_remedy_and_left_byte_identical() {
    let fx = fixture("{}");
    std::fs::create_dir_all(fx.overlay.parent().unwrap()).unwrap();
    std::fs::write(&fx.overlay, r#"{"agents":{"defaults":{"model":"theirs"}}}"#).unwrap();
    let before = std::fs::read(&fx.overlay).unwrap();
    let error = fx
        .writer
        .persist_model(DefaultScope::Local, "acme/m")
        .unwrap_err();
    assert!(error.contains("not trusted"), "{error}");
    assert!(error.contains("quecto config trust"), "{error}");
    assert_eq!(std::fs::read(&fx.overlay).unwrap(), before);
}

#[test]
fn an_explicit_selection_has_no_overlay_to_record_into() {
    let base = TempDir::new().unwrap();
    let explicit = base.path().join("explicit.json");
    std::fs::write(&explicit, "{}").unwrap();
    let handles = build_configuration_handles(&ConfigurationEnvironment {
        base_dir: base.path().to_path_buf(),
        prompt_for_trust: false,
    });
    let writer = ConfigDefaultsWriter::new(handles.patch, ConfigSelection::Explicit(explicit));
    let error = writer
        .persist_model(DefaultScope::Local, "acme/m")
        .unwrap_err();
    assert!(error.contains("no repo-local overlay"), "{error}");
}

#[test]
fn the_unbound_writer_refuses_naming_why() {
    let writer = ConfigDefaultsWriter::unavailable();
    let error = writer
        .persist_effort(DefaultScope::Global, EffortLevel::Low)
        .unwrap_err();
    assert!(error.contains("no file to record"), "{error}");
    assert!(format!("{writer:?}").contains("bound: false"));
}
