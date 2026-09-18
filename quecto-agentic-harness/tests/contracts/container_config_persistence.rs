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
//! or trusted overlay, writing nothing; `location` names the overlay file;
//! `persist` with `displace_default` removes that overlay entry's label
//! in the same write (#2035) — the other entry otherwise verbatim, the
//! merge valid throughout — and refuses to name an entry the overlay does
//! not declare, writing nothing.
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
    let receipt = port.persist("standard", &entry(true), None).unwrap();
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
    let receipt = port.persist("standard", &entry(true), None).unwrap();
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
    port.persist("mine", &entry(false), None).unwrap();
    // Trusted: the second write goes through the trust the first recorded.
    port.persist("standard", &entry(false), None).unwrap();
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
    let error = rig
        .port()
        .persist("standard", &entry(true), None)
        .unwrap_err();
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
        .persist("standard", &ContainerConfigDocument::default(), None)
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
    let error = port.persist("standard", &entry(true), None).unwrap_err();
    assert!(error.contains("no repo-local overlay"), "{error}");
    assert_eq!(port.check().unwrap_err(), error);
}

#[test]
fn existing_reads_back_the_overlays_own_entry_and_never_a_global_one() {
    let rig = Rig::new();
    let port = rig.port();
    assert_eq!(port.existing("standard").unwrap(), None, "no overlay yet");
    // A global entry of the same name is not the overlay's.
    std::fs::write(
        rig.base_dir.join("config.json"),
        serde_json::json!({"container_configs": {"standard": {
            "default": true, "create": ["/g/create.sh", "--repo", "https://global.test/g"], "cleanup": ["/g/kill.sh"]
        }}})
        .to_string(),
    )
    .unwrap();
    assert_eq!(port.existing("standard").unwrap(), None);
    // Written through the port, read back whole (the overlay's default
    // label un-defaults the global entry: one default in the merge).
    let mut ours = entry(true);
    ours.create.extend([
        "--repo".to_string(),
        "https://ours.test/r".to_string(),
        "--image".to_string(),
        "mine:1".to_string(),
    ]);
    port.persist("standard", &ours, None).unwrap();
    let existing = port.existing("standard").unwrap().unwrap();
    assert_eq!(existing, ours);
    assert_eq!(existing.create_value("--repo"), Some("https://ours.test/r"));
    assert_eq!(existing.create_value("--image"), Some("mine:1"));
    assert_eq!(port.existing("other").unwrap(), None);
}

#[test]
fn existing_is_none_while_the_overlay_is_untrusted() {
    let rig = Rig::new();
    std::fs::create_dir_all(rig.overlay().parent().unwrap()).unwrap();
    std::fs::write(
        rig.overlay(),
        serde_json::json!({"container_configs": {"standard": {
            "create": ["/x/create.sh", "--repo", "https://x.test/r"], "cleanup": ["/x/kill.sh"]
        }}})
        .to_string(),
    )
    .unwrap();
    let port = rig.port();
    // Untrusted content is never read back as "what we wrote before";
    // `check` is the refusal that stops an init here first.
    assert_eq!(port.existing("standard").unwrap(), None);
    assert!(port.check().is_err());
}

// ─── The default label moved in one write (#2035) ────────────────────────────

#[test]
fn displacing_another_overlay_entrys_label_lands_in_the_same_write_with_the_entry_verbatim() {
    let rig = Rig::new();
    // No global default: the displaced entry is the only default in the
    // merge, so any two-step write would be refused between the steps.
    let port = rig.port();
    let mut other = entry(true);
    other.create = vec![
        "/o/create.sh".into(),
        "--repo".into(),
        "https://o.test/r".into(),
    ];
    port.persist("other", &other, None).unwrap();
    let overlay_path = rig.overlay();

    let receipt = port
        .persist("standard", &entry(true), Some("other"))
        .unwrap();
    assert!(!receipt.created);
    let overlay: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&overlay_path).unwrap()).unwrap();
    let section = &overlay["container_configs"];
    assert_eq!(section["standard"]["default"], serde_json::json!(true));
    assert!(section["other"].get("default").is_none(), "{section}");
    assert_eq!(
        section["other"]["create"],
        serde_json::json!(["/o/create.sh", "--repo", "https://o.test/r"])
    );
    let effective = rig.effective();
    assert_eq!(
        effective["container_configs"]["standard"]["default"],
        serde_json::json!(true)
    );
    assert_ne!(
        effective["container_configs"]["other"]["default"],
        serde_json::json!(true)
    );
}

#[test]
fn displacing_an_entry_the_overlay_does_not_declare_is_refused_untouched() {
    let rig = Rig::new();
    let port = rig.port();
    port.persist("mine", &entry(true), None).unwrap();
    let before = std::fs::read(rig.overlay()).unwrap();
    let error = port
        .persist("standard", &entry(true), Some("ghost"))
        .unwrap_err();
    assert!(error.contains("container_configs.ghost"), "{error}");
    assert!(error.contains("declares no such entry"), "{error}");
    assert_eq!(std::fs::read(rig.overlay()).unwrap(), before);
}
