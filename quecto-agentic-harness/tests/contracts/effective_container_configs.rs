//! Contract for the `EffectiveContainerConfigs` port (#2024 S4a): the
//! container configs a launch selects from are the configuration
//! capability's *effective* ones — for the launching agent, its base file
//! with its checkout's trusted overlay merged entry-wise (an overlay
//! default un-defaults the global entries); an untrusted overlay is not
//! applied and is reported, never prompted for; an explicit file replaces
//! the layers; a launcher with no agent configuration reports no source;
//! names are sorted.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tempfile::TempDir;

use quecto::application::configuration::dto::{ConfigLayers, ConfigSelection, OverlayTrustRequest};
use quecto::application::subagents::dto::{ContainerConfigSource, ContainerConfigsError};
use quecto::application::subagents::ports::EffectiveContainerConfigs;
use quecto::composition::container_configs::build_effective_container_configs;

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

    fn under_test(&self, launching_agent: bool) -> Arc<dyn EffectiveContainerConfigs> {
        build_effective_container_configs(&self.base_dir, launching_agent.then(|| self.selection()))
    }
}

#[test]
fn the_launching_agent_reads_its_layers_with_a_trusted_overlay_merged_entry_wise() {
    let rig = Rig::new();
    rig.write_overlay(serde_json::json!({"repo": entry(true, "https://repo.test/r")}));
    rig.trust_overlay();
    let set = rig
        .under_test(true)
        .effective_container_configs(&ContainerConfigSource::LaunchingAgent)
        .unwrap();
    assert_eq!(
        set.names(),
        vec!["global".to_string(), "repo".to_string(), "zeta".to_string()]
    );
    let defaults: Vec<&str> = set
        .configs
        .iter()
        .filter(|c| c.default)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(
        defaults,
        vec!["repo"],
        "an overlay default un-defaults the global entries"
    );
    let repo = set.configs.iter().find(|c| c.name == "repo").unwrap();
    assert_eq!(
        repo.create,
        vec!["/bin/create", "--repo", "https://repo.test/r"]
    );
    assert_eq!(repo.cleanup, vec!["/bin/cleanup"]);
    assert!(set.diagnostics.is_empty());
}

#[test]
fn an_untrusted_overlay_is_not_applied_and_is_reported_without_a_prompt() {
    let rig = Rig::new();
    rig.write_overlay(serde_json::json!({"repo": entry(true, "https://repo.test/r")}));
    let set = rig
        .under_test(true)
        .effective_container_configs(&ContainerConfigSource::LaunchingAgent)
        .unwrap();
    assert_eq!(set.names(), vec!["global".to_string(), "zeta".to_string()]);
    assert!(
        set.configs
            .iter()
            .find(|c| c.name == "global")
            .unwrap()
            .default
    );
    assert_eq!(set.diagnostics.len(), 1, "{:?}", set.diagnostics);
    assert!(
        set.diagnostics[0].contains(&rig.overlay().display().to_string())
            && set.diagnostics[0].contains("quecto config trust"),
        "{:?}",
        set.diagnostics
    );
    // Reading never records trust: the overlay stays untrusted.
    let again = rig
        .under_test(true)
        .effective_container_configs(&ContainerConfigSource::LaunchingAgent)
        .unwrap();
    assert_eq!(again.diagnostics.len(), 1);
}

#[test]
fn an_explicit_file_replaces_the_layers() {
    let rig = Rig::new();
    rig.write_overlay(serde_json::json!({"repo": entry(true, "https://repo.test/r")}));
    rig.trust_overlay();
    let explicit = rig.base_dir.join("explicit.json");
    std::fs::write(
        &explicit,
        serde_json::json!({"container_configs": {"only": entry(true, "x")}}).to_string(),
    )
    .unwrap();
    let set = rig
        .under_test(true)
        .effective_container_configs(&ContainerConfigSource::Explicit(explicit))
        .unwrap();
    assert_eq!(set.names(), vec!["only".to_string()]);
    assert!(set.diagnostics.is_empty());
}

#[test]
fn a_missing_or_invalid_file_is_an_error_naming_it() {
    let rig = Rig::new();
    let missing = rig.base_dir.join("missing.json");
    let err = rig
        .under_test(true)
        .effective_container_configs(&ContainerConfigSource::Explicit(missing.clone()))
        .unwrap_err();
    match err {
        ContainerConfigsError::Invalid(reason) => {
            assert!(reason.contains(&missing.display().to_string()), "{reason}");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    let broken = rig.base_dir.join("broken.json");
    std::fs::write(&broken, "{not json").unwrap();
    let err = rig
        .under_test(true)
        .effective_container_configs(&ContainerConfigSource::Explicit(broken.clone()))
        .unwrap_err();
    assert!(
        matches!(&err, ContainerConfigsError::Invalid(reason) if reason.contains(&broken.display().to_string())),
        "{err:?}"
    );
}

#[test]
fn a_launcher_without_an_agent_configuration_reports_no_source() {
    let rig = Rig::new();
    let err = rig
        .under_test(false)
        .effective_container_configs(&ContainerConfigSource::LaunchingAgent)
        .unwrap_err();
    assert_eq!(err, ContainerConfigsError::NoSource);
    assert_eq!(
        err.to_string(),
        "container spawn requires --config so container_configs can be loaded"
    );
    let set = rig
        .under_test(false)
        .effective_container_configs(&ContainerConfigSource::Explicit(
            rig.base_dir.join("config.json"),
        ))
        .unwrap();
    assert_eq!(set.names(), vec!["global".to_string(), "zeta".to_string()]);
}

#[test]
fn an_absent_global_file_yields_the_overlay_alone_or_nothing() {
    let rig = Rig::new();
    std::fs::remove_file(rig.base_dir.join("config.json")).unwrap();
    let set = rig
        .under_test(true)
        .effective_container_configs(&ContainerConfigSource::LaunchingAgent)
        .unwrap();
    assert!(set.configs.is_empty());
    rig.write_overlay(serde_json::json!({"repo": entry(true, "https://repo.test/r")}));
    rig.trust_overlay();
    let set = rig
        .under_test(true)
        .effective_container_configs(&ContainerConfigSource::LaunchingAgent)
        .unwrap();
    assert_eq!(set.names(), vec!["repo".to_string()]);
    assert!(Path::new(&set.configs[0].create[0]).is_absolute());
}
