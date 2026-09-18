//! Contract for the `ContainerConfigPersistence` port (#2024 S4e): the
//! entry is written as `container_configs.<name>` into the selection's
//! repo-local overlay through the configuration capability's one write
//! path — the overlay created when absent, other keys and entries
//! preserved, trust recorded for exactly the bytes written (the
//! effective configuration then carries the entry), a non-default entry
//! written without the label, an untrusted overlay refused in that
//! capability's own words with nothing written, an entry the schema
//! refuses (no argv) likewise, a selection without an overlay location
//! refused; `check` gives the refusal a persist would give (untrusted
//! overlay of any content, no overlay location) and passes for an absent
//! or trusted overlay, writing nothing; `location` names the overlay file.
use std::path::PathBuf;
use std::sync::Arc;

use tempfile::TempDir;

use quecto::application::configuration::dto::{ConfigLayers, ConfigSelection};
use quecto::application::environments::dto::ContainerConfigDocument;
use quecto::application::environments::ports::ContainerConfigPersistence;
use quecto::composition::standard_container::build_container_config_persistence;

struct Rig {
    _dir: TempDir,
    base_dir: PathBuf,
    checkout: PathBuf,
}

impl Rig {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let base_dir = dir.path().join("base");
        let checkout = dir.path().join("checkout");
        std::fs::create_dir_all(&base_dir).unwrap();
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(base_dir.join("config.json"), "{}").unwrap();
        Self {
            _dir: dir,
            base_dir,
            checkout,
        }
    }

    fn overlay(&self) -> PathBuf {
        self.checkout.join(".quecto/config.json")
    }

    fn selection(&self) -> ConfigSelection {
        ConfigSelection::Layered(ConfigLayers {
            global: self.base_dir.join("config.json"),
            overlay: Some(self.overlay()),
            legacy_local: None,
        })
    }

    fn port(&self) -> Arc<dyn ContainerConfigPersistence> {
        build_container_config_persistence(&self.base_dir, &self.selection())
    }

    fn effective(&self) -> serde_json::Value {
        let handles = quecto::composition::configuration::build_configuration_handles(
            &quecto::interface::cli::configuration_handles::ConfigurationEnvironment {
                base_dir: self.base_dir.clone(),
                prompt_for_trust: false,
            },
        );
        let effective = handles.resolve.execute(&self.selection()).unwrap();
        assert!(
            effective.sources.diagnostics().is_empty(),
            "{:?}",
            effective.sources
        );
        effective.document
    }
}

fn entry(default: bool) -> ContainerConfigDocument {
    ContainerConfigDocument {
        default,
        create: vec!["/s/create.sh".into(), "--state-dir".into(), "/st".into()],
        exec: vec!["/s/exec.sh".into()],
        inspect: vec!["/s/inspect.sh".into()],
        kill: vec!["/s/kill.sh".into(), "--op".into(), "kill".into()],
        cleanup: vec!["/s/kill.sh".into(), "--op".into(), "cleanup".into()],
    }
}

#[test]
fn the_entry_lands_in_a_created_trusted_overlay_and_the_effective_configuration_carries_it() {
    let rig = Rig::new();
    let port = rig.port();
    assert_eq!(port.location(), Some(rig.overlay()));
    let receipt = port.persist("standard", &entry(true)).unwrap();
    assert_eq!(receipt.path, rig.overlay());
    assert!(receipt.created);
    let effective = rig.effective();
    let written = &effective["container_configs"]["standard"];
    assert_eq!(written["default"], serde_json::json!(true));
    assert_eq!(
        written["create"],
        serde_json::json!(["/s/create.sh", "--state-dir", "/st"])
    );
    assert_eq!(
        written["cleanup"],
        serde_json::json!(["/s/kill.sh", "--op", "cleanup"])
    );
    // A second persist of the same entry is a no-op on disk.
    let before = std::fs::read(rig.overlay()).unwrap();
    let receipt = port.persist("standard", &entry(true)).unwrap();
    assert!(!receipt.created);
    assert_eq!(std::fs::read(rig.overlay()).unwrap(), before);
}

#[test]
fn other_keys_and_entries_survive_and_a_non_default_entry_carries_no_label() {
    let rig = Rig::new();
    std::fs::write(
        rig.base_dir.join("config.json"),
        serde_json::json!({"container_configs": {"other": {"default": true, "create": ["/c"], "cleanup": ["/k"]}}}).to_string(),
    )
    .unwrap();
    let port = rig.port();
    port.persist("mine", &entry(false)).unwrap();
    // Trusted: the second write goes through the trust the first recorded.
    port.persist("standard", &entry(false)).unwrap();
    let overlay: serde_json::Value =
        serde_json::from_slice(&std::fs::read(rig.overlay()).unwrap()).unwrap();
    assert!(overlay["container_configs"]["mine"].is_object());
    assert!(
        overlay["container_configs"]["standard"]
            .get("default")
            .is_none()
    );
    let effective = rig.effective();
    assert_eq!(
        effective["container_configs"]["other"]["default"],
        serde_json::json!(true)
    );
    assert!(effective["container_configs"]["standard"].is_object());
}

#[test]
fn an_untrusted_overlay_is_refused_untouched() {
    let rig = Rig::new();
    std::fs::create_dir_all(rig.overlay().parent().unwrap()).unwrap();
    let hand_written = serde_json::json!({"agents": {"defaults": {"model": "m"}}}).to_string();
    std::fs::write(rig.overlay(), &hand_written).unwrap();
    let error = rig.port().persist("standard", &entry(true)).unwrap_err();
    assert!(error.contains("is not trusted"), "{error}");
    assert!(error.contains("quecto config trust"), "{error}");
    assert_eq!(
        std::fs::read_to_string(rig.overlay()).unwrap(),
        hand_written
    );
}

#[test]
fn an_entry_the_schema_refuses_is_refused_with_nothing_written() {
    let rig = Rig::new();
    let error = rig
        .port()
        .persist("standard", &ContainerConfigDocument::default())
        .unwrap_err();
    assert!(error.contains("refusing to write"), "{error}");
    assert!(!rig.overlay().exists(), "nothing written");
}

#[test]
fn a_selection_without_an_overlay_location_is_refused() {
    let rig = Rig::new();
    let explicit = ConfigSelection::Explicit(rig.base_dir.join("config.json"));
    let port = build_container_config_persistence(&rig.base_dir, &explicit);
    assert_eq!(port.location(), None);
    let error = port.persist("standard", &entry(true)).unwrap_err();
    assert!(error.contains("no repo-local overlay"), "{error}");
    assert_eq!(port.check().unwrap_err(), error);
}
