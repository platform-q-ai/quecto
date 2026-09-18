//! Contract for the `ContainerConfigLookup` port (#2024 S4b): the doctor's
//! target is resolved exactly as a launch resolves it — the labelled
//! default of the effective configuration (a trusted overlay's default
//! un-defaults the global entries), or the named entry; an unknown name
//! is refused enumerating the live names; an untrusted overlay that
//! declares `container_configs` withholds the default and its diagnostic
//! is the reason; a launcher composed without a selection reports so.
use std::path::PathBuf;
use std::sync::Arc;

use tempfile::TempDir;

use quecto::application::configuration::dto::{ConfigLayers, ConfigSelection, OverlayTrustRequest};
use quecto::application::environments::dto::ContainerRuntimeTarget;
use quecto::application::environments::ports::ContainerConfigLookup;
use quecto::composition::environments::build_container_config_lookup;

fn entry(default: bool, marker: &str) -> serde_json::Value {
    let mut entry = serde_json::json!({
        "create": ["/bin/create", "--repo", marker],
        "cleanup": ["/bin/cleanup"],
    });
    if default {
        entry["default"] = serde_json::json!(true);
    }
    entry
}

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
        std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
        std::fs::write(
            base_dir.join("config.json"),
            serde_json::json!({"container_configs": {
                "zeta": entry(false, "z"),
                "global": entry(true, "https://global.test/g"),
            }})
            .to_string(),
        )
        .unwrap();
        Self {
            _dir: dir,
            base_dir,
            checkout,
        }
    }

    fn overlay(&self) -> PathBuf {
        self.checkout.join(".quecto/config.json")
    }

    fn write_overlay(&self, configs: serde_json::Value) {
        std::fs::write(
            self.overlay(),
            serde_json::json!({"container_configs": configs}).to_string(),
        )
        .unwrap();
    }

    fn trust_overlay(&self) {
        quecto::composition::configuration::build_configuration_handles(
            &quecto::interface::cli::configuration_handles::ConfigurationEnvironment {
                base_dir: self.base_dir.clone(),
                prompt_for_trust: false,
            },
        )
        .trust
        .execute(OverlayTrustRequest {
            path: self.overlay(),
        })
        .unwrap();
    }

    fn selection(&self) -> ConfigSelection {
        ConfigSelection::Layered(ConfigLayers {
            global: self.base_dir.join("config.json"),
            overlay: Some(self.overlay()),
            legacy_local: None,
        })
    }

    fn under_test(&self, composed: bool) -> Arc<dyn ContainerConfigLookup> {
        build_container_config_lookup(&self.base_dir, composed.then(|| self.selection()))
    }
}

fn named(name: &str) -> ContainerRuntimeTarget {
    ContainerRuntimeTarget {
        name: Some(name.to_string()),
    }
}

#[test]
fn the_default_target_is_the_effective_default_with_the_overlay_applied() {
    let rig = Rig::new();
    rig.write_overlay(serde_json::json!({"repo": entry(true, "https://repo.test/r")}));
    rig.trust_overlay();
    let config = rig
        .under_test(true)
        .lookup(&ContainerRuntimeTarget::default())
        .unwrap();
    assert_eq!(config.name, "repo");
    assert_eq!(
        config.create,
        vec!["/bin/create", "--repo", "https://repo.test/r"]
    );
    assert!(config.diagnostics.is_empty());
}

#[test]
fn without_an_overlay_the_global_default_is_the_target() {
    let rig = Rig::new();
    let config = rig
        .under_test(true)
        .lookup(&ContainerRuntimeTarget::default())
        .unwrap();
    assert_eq!(config.name, "global");
    assert_eq!(
        config.create,
        vec!["/bin/create", "--repo", "https://global.test/g"]
    );
}

#[test]
fn a_named_target_selects_that_entry_and_an_unknown_name_enumerates_the_live_names() {
    let rig = Rig::new();
    let lookup = rig.under_test(true);
    assert_eq!(lookup.lookup(&named("zeta")).unwrap().name, "zeta");
    let error = lookup.lookup(&named("nope")).unwrap_err();
    assert!(
        error.contains(
            "unknown container config 'nope' (available container configs: global, zeta)"
        ),
        "{error}"
    );
}

#[test]
fn an_untrusted_overlay_declaring_container_configs_withholds_the_default_and_names_itself() {
    let rig = Rig::new();
    rig.write_overlay(serde_json::json!({"repo": entry(true, "https://repo.test/r")}));
    let lookup = rig.under_test(true);
    let error = lookup
        .lookup(&ContainerRuntimeTarget::default())
        .unwrap_err();
    assert!(error.contains("was not applied"), "{error}");
    assert!(
        error.contains(&rig.overlay().display().to_string()),
        "{error}"
    );
    // A named global entry still resolves, carrying the diagnostic.
    let config = lookup.lookup(&named("global")).unwrap();
    assert_eq!(config.name, "global");
    assert_eq!(config.diagnostics.len(), 1, "{:?}", config.diagnostics);
    assert!(config.diagnostics[0].contains("is not trusted"));
}

#[test]
fn a_lookup_composed_without_a_selection_reports_so() {
    let rig = Rig::new();
    let error = rig
        .under_test(false)
        .lookup(&ContainerRuntimeTarget::default())
        .unwrap_err();
    assert!(
        !error.is_empty(),
        "the refusal is the selection's own account"
    );
    assert!(
        !error.contains("global"),
        "no entry is guessed without a selection: {error}"
    );
}
