use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::{build_container_config_selection, build_effective_container_configs};
use crate::application::configuration::dto::{ConfigLayers, ConfigSelection};
use crate::application::subagents::dto::{
    ContainerConfigSource, ContainerConfigsError, SelectContainerConfigRequest,
};

fn entry(default: bool, repo: &str) -> serde_json::Value {
    let mut entry = serde_json::json!({
        "create": ["/bin/create", "--repo", repo],
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
            serde_json::json!({"container_configs": {"global": entry(true, "https://global.test/g")}})
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
        let handles = crate::composition::configuration::build_configuration_handles(
            &crate::interface::cli::configuration_handles::ConfigurationEnvironment {
                base_dir: self.base_dir.clone(),
                prompt_for_trust: false,
            },
        );
        handles
            .trust
            .execute(
                crate::application::configuration::dto::OverlayTrustRequest {
                    path: self.overlay(),
                },
            )
            .unwrap();
    }

    fn selection(&self) -> ConfigSelection {
        ConfigSelection::Layered(ConfigLayers {
            global: self.base_dir.join("config.json"),
            overlay: Some(self.overlay()),
            legacy_local: None,
        })
    }

    fn select(&self, name: Option<&str>) -> Result<String, String> {
        let select = build_container_config_selection(&self.base_dir, Some(self.selection()));
        select
            .execute(&SelectContainerConfigRequest {
                source: ContainerConfigSource::LaunchingAgent,
                name: name.map(String::from),
            })
            .map(|selected| selected.config.name)
            .map_err(|error| error.to_string())
    }
}

#[test]
fn a_trusted_overlay_default_un_defaults_the_global_entry_and_is_selected() {
    let rig = Rig::new();
    rig.write_overlay(serde_json::json!({"repo": entry(true, "https://repo.test/r")}));
    rig.trust_overlay();
    assert_eq!(rig.select(None).unwrap(), "repo");
    // Both entries remain selectable by name.
    assert_eq!(rig.select(Some("global")).unwrap(), "global");
    let configs = build_effective_container_configs(&rig.base_dir, Some(rig.selection()))
        .effective_container_configs(&ContainerConfigSource::LaunchingAgent)
        .unwrap();
    assert_eq!(
        configs.names(),
        vec!["global".to_string(), "repo".to_string()]
    );
    assert!(configs.diagnostics.is_empty());
    let repo = configs.configs.iter().find(|c| c.name == "repo").unwrap();
    assert_eq!(
        repo.create,
        vec!["/bin/create", "--repo", "https://repo.test/r"]
    );
    assert!(
        !configs
            .configs
            .iter()
            .find(|c| c.name == "global")
            .unwrap()
            .default
    );
}

#[test]
fn an_untrusted_overlay_is_not_applied_and_travels_as_the_configuration_diagnostic() {
    let rig = Rig::new();
    rig.write_overlay(serde_json::json!({"repo": entry(true, "https://repo.test/r")}));
    let configs = build_effective_container_configs(&rig.base_dir, Some(rig.selection()))
        .effective_container_configs(&ContainerConfigSource::LaunchingAgent)
        .unwrap();
    assert_eq!(configs.names(), vec!["global".to_string()]);
    assert_eq!(configs.diagnostics.len(), 1, "{:?}", configs.diagnostics);
    assert!(
        configs.diagnostics[0].contains("is not trusted")
            && configs.diagnostics[0].contains("quecto config trust"),
        "{:?}",
        configs.diagnostics
    );
    assert!(configs.overlay_withheld);
    // The implicit default is refused while the overlay is withheld; a
    // name launches from the global set.
    let refused = rig.select(None).unwrap_err();
    assert!(
        refused.starts_with("container: true refused")
            && refused.contains(&rig.overlay().display().to_string())
            && refused.contains("quecto config trust"),
        "{refused}"
    );
    assert_eq!(rig.select(Some("global")).unwrap(), "global");
}

#[test]
fn an_untrusted_overlay_without_container_configs_warns_and_keeps_the_global_default() {
    // A checkout whose overlay only pins `agents.defaults` (#2024 S2)
    // cannot have changed the default container config, so a fresh clone
    // or non-tty run is not refused: the global default launches and the
    // untrusted-overlay diagnostic travels as a warning.
    let rig = Rig::new();
    std::fs::write(
        rig.overlay(),
        serde_json::json!({"agents": {"defaults": {"model": "pinned"}}}).to_string(),
    )
    .unwrap();
    let configs = build_effective_container_configs(&rig.base_dir, Some(rig.selection()))
        .effective_container_configs(&ContainerConfigSource::LaunchingAgent)
        .unwrap();
    assert!(!configs.overlay_withheld);
    assert_eq!(configs.diagnostics.len(), 1, "{:?}", configs.diagnostics);
    assert!(
        configs.diagnostics[0].contains("is not trusted"),
        "{:?}",
        configs.diagnostics
    );
    assert_eq!(rig.select(None).unwrap(), "global");
}

#[test]
fn an_untrusted_overlay_that_is_refused_or_unparseable_withholds_the_default() {
    // Content the checks refuse (here: unparseable) and a document the
    // store refuses outright (a symlink) declare nothing knowable, so the
    // default stays unknown and `container: true` is refused.
    let rig = Rig::new();
    std::fs::write(rig.overlay(), "{not json").unwrap();
    let configs = build_effective_container_configs(&rig.base_dir, Some(rig.selection()))
        .effective_container_configs(&ContainerConfigSource::LaunchingAgent)
        .unwrap();
    assert!(configs.overlay_withheld, "{:?}", configs.diagnostics);
    assert!(
        rig.select(None)
            .unwrap_err()
            .starts_with("container: true refused")
    );

    std::fs::remove_file(rig.overlay()).unwrap();
    let target = rig.checkout.join("elsewhere.json");
    std::fs::write(
        &target,
        serde_json::json!({"agents": {"defaults": {"model": "pinned"}}}).to_string(),
    )
    .unwrap();
    std::os::unix::fs::symlink(&target, rig.overlay()).unwrap();
    let configs = build_effective_container_configs(&rig.base_dir, Some(rig.selection()))
        .effective_container_configs(&ContainerConfigSource::LaunchingAgent)
        .unwrap();
    assert!(configs.overlay_withheld, "{:?}", configs.diagnostics);
    assert!(
        configs.diagnostics[0].contains("was not applied"),
        "{:?}",
        configs.diagnostics
    );
    assert!(
        rig.select(None)
            .unwrap_err()
            .starts_with("container: true refused")
    );
}

#[test]
fn an_explicit_file_replaces_the_layers_and_ignores_the_overlay() {
    let rig = Rig::new();
    rig.write_overlay(serde_json::json!({"repo": entry(true, "https://repo.test/r")}));
    rig.trust_overlay();
    let explicit = rig.base_dir.join("explicit.json");
    std::fs::write(
        &explicit,
        serde_json::json!({"container_configs": {"only": entry(true, "https://x.test/x")}})
            .to_string(),
    )
    .unwrap();
    let select = build_container_config_selection(&rig.base_dir, Some(rig.selection()));
    let selected = select
        .execute(&SelectContainerConfigRequest {
            source: ContainerConfigSource::Explicit(explicit.clone()),
            name: None,
        })
        .unwrap();
    assert_eq!(selected.config.name, "only");
    // A named file that does not exist is an error, never a quiet default.
    let missing = select
        .execute(&SelectContainerConfigRequest {
            source: ContainerConfigSource::Explicit(rig.base_dir.join("missing.json")),
            name: None,
        })
        .unwrap_err()
        .to_string();
    // Load errors keep the section prefix a spawn caller has always seen
    // (parity with the retired loader): the failure is about the
    // container configuration this launch needed.
    assert!(
        missing.starts_with("invalid container_configs configuration: config not found: "),
        "{missing}"
    );
    let broken = rig.base_dir.join("broken.json");
    std::fs::write(&broken, "{not json").unwrap();
    let parse = select
        .execute(&SelectContainerConfigRequest {
            source: ContainerConfigSource::Explicit(broken.clone()),
            name: None,
        })
        .unwrap_err()
        .to_string();
    assert!(
        parse.starts_with("invalid container_configs configuration: failed to load config ")
            && parse.contains(&broken.display().to_string()),
        "{parse}"
    );
}

#[test]
fn a_trusted_overlay_whose_merge_is_invalid_fails_naming_both_files_without_a_prefix_stutter() {
    let rig = Rig::new();
    rig.write_overlay(serde_json::json!({
        "r": entry(true, "https://repo.test/r"),
        "s": entry(true, "https://repo.test/s"),
    }));
    rig.trust_overlay();
    let err = rig.select(None).unwrap_err();
    assert!(
        err.starts_with("failed to load config ")
            && err.contains(&rig.overlay().display().to_string())
            && err.contains("merged over")
            && err.contains("invalid container_configs: multiple container configs are labeled"),
        "{err}"
    );
    assert!(!err.contains("configuration: failed"), "no stutter: {err}");
    // A name does not rescue an invalid merge: nothing is selected from it.
    let named = rig.select(Some("global")).unwrap_err();
    assert_eq!(named, err);
}

#[test]
fn a_launcher_without_an_agent_configuration_reports_the_no_source_error() {
    let rig = Rig::new();
    let err = build_effective_container_configs(&rig.base_dir, None)
        .effective_container_configs(&ContainerConfigSource::LaunchingAgent)
        .unwrap_err();
    assert_eq!(err, ContainerConfigsError::NoSource);
    // An explicit file still works for such a launcher.
    let explicit = rig.base_dir.join("config.json");
    let configs = build_effective_container_configs(&rig.base_dir, None)
        .effective_container_configs(&ContainerConfigSource::Explicit(explicit))
        .unwrap();
    assert_eq!(configs.names(), vec!["global".to_string()]);
}

#[test]
fn a_merge_that_leaves_no_default_is_refused_naming_both_files() {
    let rig = Rig::new();
    std::fs::write(
        rig.base_dir.join("config.json"),
        serde_json::json!({"container_configs": {"global": entry(false, "https://global.test/g")}})
            .to_string(),
    )
    .unwrap();
    // The global file alone is invalid (no default): the load fails there.
    let err = rig.select(None).unwrap_err();
    assert!(err.contains("no container config is labeled"), "{err}");
    assert!(
        err.contains(&rig.base_dir.join("config.json").display().to_string()),
        "{err}"
    );
}

#[test]
fn realization_ignores_the_process_environment() {
    // The adapter realizes without `QUECTO_*` overrides: the argv a spawn
    // executes never depends on this process's environment.
    let rig = Rig::new();
    let configs = build_effective_container_configs(&rig.base_dir, Some(rig.selection()))
        .effective_container_configs(&ContainerConfigSource::LaunchingAgent)
        .unwrap();
    assert_eq!(configs.names(), vec!["global".to_string()]);
    assert!(Path::new(&configs.configs[0].create[0]).is_absolute());
}

#[test]
fn the_composed_handles_and_port_adapters_describe_themselves_without_naming_their_inputs() {
    let rig = Rig::new();
    let handles = super::build_agent_container_config_handles(&rig.base_dir, &rig.selection());
    assert_eq!(
        format!("{handles:?}"),
        "ContainerConfigHandles { selection: SelectContainerConfig { .. }, roster: ListContainerConfigs { .. } }"
    );
    let doctor =
        super::super::environments::build_container_doctor(&rig.base_dir, &rig.selection());
    assert!(
        format!("{doctor:?}").starts_with("DiagnoseContainerRuntime"),
        "{doctor:?}"
    );
    let admission = super::super::admission::build_admission_handles();
    assert_eq!(format!("{admission:?}"), "AdmissionHandles { .. }");
    assert_eq!(
        format!("{:?}", admission.inspect),
        "InspectAuthority { .. }"
    );
    assert_eq!(format!("{:?}", admission.reset), "ResetAuthority { .. }");
    assert!(
        format!("{:?}", admission.install).starts_with("InstallAuthorityService"),
        "{:?}",
        admission.install
    );
    assert!(
        format!("{:?}", admission.uninstall).starts_with("UninstallAuthorityService"),
        "{:?}",
        admission.uninstall
    );
}
