use std::path::{Path, PathBuf};

use super::{
    OverlayContainerConfigWriter, build_container_asset_store, build_container_config_persistence,
    build_container_status, build_standard_container_init, build_workspace_origin, document_entry,
    entry_document,
};
use crate::application::configuration::dto::{ConfigLayers, ConfigSelection};
use crate::application::environments::dto::{
    AssetState, ContainerConfigDocument, STANDARD_CONTAINER_DIR, StandardContainerRequest,
};
use crate::application::environments::ports::ContainerConfigPersistence;

struct Rig {
    _dir: tempfile::TempDir,
    base_dir: PathBuf,
    checkout: PathBuf,
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let base_dir = dir.path().join("base");
        let checkout = dir.path().join("checkout");
        std::fs::create_dir_all(&base_dir).unwrap();
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(base_dir.join("config.json"), "{}").unwrap();
        Self {
            _dir: dir,
            base_dir: base_dir.canonicalize().unwrap(),
            checkout: checkout.canonicalize().unwrap(),
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
}

fn entry(default: bool) -> ContainerConfigDocument {
    ContainerConfigDocument {
        default,
        create: vec!["/s/create.sh".into(), "--repo".into(), "https://x/y".into()],
        exec: vec!["/s/exec.sh".into()],
        inspect: vec!["/s/inspect.sh".into()],
        kill: vec!["/s/kill.sh".into(), "--op".into(), "kill".into()],
        cleanup: vec!["/s/kill.sh".into(), "--op".into(), "cleanup".into()],
    }
}

#[test]
fn the_entry_document_spells_the_label_only_when_set_and_reads_back_leniently() {
    let document = entry_document(&entry(true));
    assert_eq!(document["default"], true);
    assert_eq!(document["create"][2], "https://x/y");
    assert_eq!(document["cleanup"][2], "cleanup");
    assert_eq!(document_entry(&document), entry(true));

    let unlabelled = entry_document(&entry(false));
    assert!(unlabelled.get("default").is_none(), "{unlabelled}");
    assert_eq!(document_entry(&unlabelled), entry(false));

    // A hand-written overlay: a non-string argv element and a missing
    // argv are dropped, never a refusal (the schema judged it on load).
    let lenient = document_entry(&serde_json::json!({
        "default": "yes",
        "create": ["/c", 7, "--repo"],
        "kill": "not-an-array"
    }));
    assert_eq!(
        lenient,
        ContainerConfigDocument {
            default: false,
            create: vec!["/c".into(), "--repo".into()],
            ..Default::default()
        }
    );
}

#[test]
fn the_overlay_writer_checks_persists_reads_back_and_names_its_location() {
    let rig = Rig::new();
    let port = build_container_config_persistence(&rig.base_dir, &rig.selection());
    assert_eq!(port.location(), Some(rig.overlay()));
    port.check().unwrap();
    assert_eq!(port.existing("standard").unwrap(), None);

    let receipt = port.persist("standard", &entry(true)).unwrap();
    assert_eq!(receipt.path, rig.overlay());
    assert!(receipt.created);
    assert_eq!(port.existing("standard").unwrap(), Some(entry(true)));
    assert_eq!(port.existing("other").unwrap(), None);

    let mut changed = entry(true);
    changed.create[2] = "https://x/z".into();
    let receipt = port.persist("standard", &changed).unwrap();
    assert!(!receipt.created);
    assert_eq!(port.existing("standard").unwrap(), Some(changed));

    // The write goes through the configuration capability's own
    // validation: an entry set without a default is its refusal, in its
    // words, and the overlay keeps the entry it had.
    let refusal = port.persist("standard", &entry(false)).unwrap_err();
    assert!(
        refusal.contains("no container config is labeled"),
        "{refusal}"
    );
    assert_eq!(
        port.existing("standard").unwrap().map(|e| e.default),
        Some(true)
    );
}

#[test]
fn a_non_default_entry_is_written_without_the_label_beside_the_global_default() {
    let rig = Rig::new();
    std::fs::write(
        rig.base_dir.join("config.json"),
        r#"{"container_configs":{"corp":{"default":true,"create":["/corp/create.sh"]}}}"#,
    )
    .unwrap();
    let port = build_container_config_persistence(&rig.base_dir, &rig.selection());
    port.persist("standard", &entry(false)).unwrap();
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(rig.overlay()).unwrap()).unwrap();
    assert!(
        written["container_configs"]["standard"]
            .get("default")
            .is_none(),
        "{written}"
    );
    assert_eq!(port.existing("standard").unwrap(), Some(entry(false)));
}

#[test]
fn the_overlay_writer_refuses_an_untrusted_overlay_and_a_selection_without_a_location() {
    let rig = Rig::new();
    std::fs::create_dir_all(rig.checkout.join(".quecto")).unwrap();
    std::fs::write(
        rig.overlay(),
        r#"{"container_configs":{"x":{"create":["/x"]}}}"#,
    )
    .unwrap();
    let port = build_container_config_persistence(&rig.base_dir, &rig.selection());
    let refusal = port.check().unwrap_err();
    assert!(refusal.contains("not trusted"), "{refusal}");
    let refusal = port.persist("standard", &entry(true)).unwrap_err();
    assert!(refusal.contains("not trusted"), "{refusal}");
    assert!(
        !std::fs::read_to_string(rig.overlay())
            .unwrap()
            .contains("standard")
    );

    let explicit = ConfigSelection::Explicit(rig.base_dir.join("config.json"));
    let handles = super::super::configuration::build_configuration_handles(
        &crate::interface::cli::configuration_handles::ConfigurationEnvironment {
            base_dir: rig.base_dir.clone(),
            prompt_for_trust: false,
        },
    );
    let port = OverlayContainerConfigWriter::new(handles.patch, handles.resolve, explicit);
    assert!(
        format!("{port:?}").starts_with("OverlayContainerConfigWriter { selection:"),
        "{port:?}"
    );
    assert_eq!(port.location(), None);
    let refusal = port.check().unwrap_err();
    assert!(refusal.contains("overlay"), "{refusal}");
    let refusal = port.persist("standard", &entry(true)).unwrap_err();
    assert!(refusal.contains("overlay"), "{refusal}");
    assert_eq!(port.existing("standard").unwrap(), None);
}

#[test]
fn the_composed_status_and_init_share_the_layers_over_a_checkout_without_git() {
    let rig = Rig::new();
    let status = build_container_status(&rig.base_dir, &rig.selection());
    let before = status.execute(&rig.checkout);
    assert_eq!(before.entry, None);
    assert!(before.assets.iter().all(|(_, s)| *s == AssetState::Missing));
    assert!(!before.healthy());
    assert!(format!("{status:?}").starts_with("ContainerStatus"));

    let init = build_standard_container_init(&rig.base_dir, &rig.selection());
    assert!(format!("{init:?}").contains("base_dir"), "{init:?}");
    let report = init
        .execute(&StandardContainerRequest {
            project: rig.checkout.clone(),
            repository: None,
            image: None,
            dry_run: false,
            refresh: false,
        })
        .unwrap();
    assert_eq!(report.written.len(), 5);
    assert_eq!(report.assets_dir, rig.checkout.join(STANDARD_CONTAINER_DIR));

    // The entry is now in the effective set the status reads; the image
    // check it would ask of the real preflight is exercised through the
    // CLI over a stub preflight (`container_setup_e2e_tests`), never by
    // running the materialised script here.
    let roster = super::super::container_configs::build_container_config_roster(
        &rig.base_dir,
        Some(rig.selection()),
    )
    .roster()
    .unwrap();
    let entry = roster
        .configs
        .iter()
        .find(|entry| entry.name == "standard")
        .expect("the standard entry");
    assert_eq!(entry.repository, None, "no git, no origin: a sandbox");
    assert!(entry.default);
    assert!(!roster.overlay_withheld);
}

#[test]
fn the_port_adapters_for_the_contract_suite_are_the_real_ones() {
    let store = build_container_asset_store();
    assert_eq!(store.catalogue().assets.len(), 5);
    let origin = build_workspace_origin();
    let dir = tempfile::TempDir::new().unwrap();
    assert_eq!(origin.origin(dir.path()).unwrap(), None);
    assert_eq!(origin.toplevel(Path::new("/")).unwrap(), None);
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_in_the_overlays_place_is_refused_in_the_patchs_words() {
    let rig = Rig::new();
    std::fs::create_dir_all(rig.checkout.join(".quecto")).unwrap();
    let elsewhere = rig.checkout.join("elsewhere.json");
    std::fs::write(&elsewhere, "{}").unwrap();
    std::os::unix::fs::symlink(&elsewhere, rig.overlay()).unwrap();
    let port = build_container_config_persistence(&rig.base_dir, &rig.selection());
    let refusal = port.check().unwrap_err();
    assert!(refusal.contains("symbolic link"), "{refusal}");
    assert!(
        refusal.contains(&rig.overlay().display().to_string()),
        "{refusal}"
    );
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "{}");
}
