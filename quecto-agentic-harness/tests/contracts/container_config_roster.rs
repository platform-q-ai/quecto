//! Contract for the `ContainerConfigRoster` port (#2024 S4c): the
//! effective container-config set of the launching agent, read exactly as
//! a launch reads it — the global file with the checkout's trusted overlay
//! merged entry-wise; an overlay entry is reported as declared by the
//! overlay (repo-bound), a global one as global, a same-name overlay entry
//! shadows the global one and is repo-bound; the repository is the create
//! argv's `--repo` with any URL userinfo redacted, `None` for a sandbox;
//! an untrusted overlay that
//! declares `container_configs` reports the global set as withheld with
//! the trust diagnostic; a roster composed without a selection reports so;
//! `joinable` follows the `exec` argv; the revision token changes when a
//! layer or the trust record is written and is stable otherwise.
use std::path::PathBuf;
use std::sync::Arc;

use tempfile::TempDir;

use quecto::application::configuration::dto::{ConfigLayers, ConfigSelection, OverlayTrustRequest};
use quecto::application::environments::dto::ContainerConfigLayer;
use quecto::application::environments::ports::ContainerConfigRoster;
use quecto::composition::container_configs::build_container_config_roster;

fn entry(default: bool, repo: Option<&str>) -> serde_json::Value {
    let mut create = vec!["/bin/create".to_string()];
    if let Some(repo) = repo {
        create.push("--repo".into());
        create.push(repo.into());
    }
    let mut entry = serde_json::json!({"create": create, "cleanup": ["/bin/cleanup"]});
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
                "sandbox": entry(false, None),
                "global": entry(true, Some("https://global.test/g")),
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

    fn under_test(&self, composed: bool) -> Arc<dyn ContainerConfigRoster> {
        build_container_config_roster(&self.base_dir, composed.then(|| self.selection()))
    }
}

#[test]
fn overlay_entries_are_repo_bound_and_shadow_global_ones() {
    let rig = Rig::new();
    rig.write_overlay(serde_json::json!({
        "repo": entry(true, Some("https://repo.test/r")),
        "global": entry(false, Some("https://repo.test/shadow")),
    }));
    rig.trust_overlay();
    let report = rig.under_test(true).roster().unwrap();
    assert!(!report.overlay_withheld);
    assert!(report.diagnostics.is_empty());
    let by_name = |name: &str| {
        report
            .configs
            .iter()
            .find(|e| e.name == name)
            .unwrap_or_else(|| panic!("no {name}: {:?}", report.configs))
    };
    let repo = by_name("repo");
    assert!(repo.default);
    assert_eq!(repo.layer, ContainerConfigLayer::Overlay);
    assert_eq!(repo.repository.as_deref(), Some("https://repo.test/r"));
    let shadowed = by_name("global");
    assert!(!shadowed.default, "the overlay default un-defaults it");
    assert_eq!(shadowed.layer, ContainerConfigLayer::Overlay);
    assert_eq!(
        shadowed.repository.as_deref(),
        Some("https://repo.test/shadow")
    );
    let sandbox = by_name("sandbox");
    assert_eq!(sandbox.layer, ContainerConfigLayer::Global);
    assert_eq!(sandbox.repository, None);
}

#[test]
fn without_an_overlay_every_entry_is_global_and_the_labelled_default_is_reported() {
    let rig = Rig::new();
    let report = rig.under_test(true).roster().unwrap();
    assert!(!report.overlay_withheld);
    assert_eq!(report.configs.len(), 2);
    assert!(
        report
            .configs
            .iter()
            .all(|e| e.layer == ContainerConfigLayer::Global)
    );
    let default: Vec<&str> = report
        .configs
        .iter()
        .filter(|e| e.default)
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(default, ["global"]);
}

#[test]
fn an_untrusted_overlay_declaring_container_configs_is_withheld_with_the_trust_diagnostic() {
    let rig = Rig::new();
    rig.write_overlay(serde_json::json!({"repo": entry(true, Some("https://repo.test/r"))}));
    let report = rig.under_test(true).roster().unwrap();
    assert!(report.overlay_withheld);
    assert!(report.configs.iter().all(|e| e.name != "repo"));
    assert_eq!(report.diagnostics.len(), 1, "{:?}", report.diagnostics);
    assert!(report.diagnostics[0].contains(&rig.overlay().display().to_string()));
    assert!(report.diagnostics[0].contains("quecto config trust"));
}

#[test]
fn an_untrusted_overlay_that_cannot_change_the_set_is_a_warning_only() {
    let rig = Rig::new();
    std::fs::write(
        rig.overlay(),
        serde_json::json!({"agents": {"defaults": {"model": "pinned"}}}).to_string(),
    )
    .unwrap();
    let report = rig.under_test(true).roster().unwrap();
    assert!(!report.overlay_withheld);
    assert_eq!(report.diagnostics.len(), 1);
    assert!(report.configs.iter().any(|e| e.default));
}

#[test]
fn an_invalid_global_file_is_the_error() {
    let rig = Rig::new();
    std::fs::write(rig.base_dir.join("config.json"), "{not json").unwrap();
    let error = rig.under_test(true).roster().unwrap_err();
    assert!(error.contains("config.json"), "{error}");
}

#[test]
fn a_roster_composed_without_a_selection_reports_so() {
    let rig = Rig::new();
    let error = rig.under_test(false).roster().unwrap_err();
    assert_eq!(
        error,
        "container spawn requires --config so container_configs can be loaded"
    );
}

#[test]
fn a_repo_url_with_userinfo_is_reported_with_the_token_redacted() {
    let rig = Rig::new();
    rig.write_overlay(serde_json::json!({
        "secret": entry(true, Some("https://user:ghp_secret@host.test/x/y.git")),
    }));
    rig.trust_overlay();
    let report = rig.under_test(true).roster().unwrap();
    let secret = report
        .configs
        .iter()
        .find(|e| e.name == "secret")
        .unwrap_or_else(|| panic!("no secret: {:?}", report.configs));
    assert_eq!(
        secret.repository.as_deref(),
        Some("https://***@host.test/x/y.git")
    );
    let whole = format!("{report:?}");
    assert!(!whole.contains("ghp_secret"), "token leaked: {whole}");
    assert!(!whole.contains("user:"), "userinfo leaked: {whole}");
}

#[test]
fn joinable_follows_the_exec_argv() {
    let rig = Rig::new();
    let mut with_exec = entry(false, None);
    with_exec["exec"] = serde_json::json!(["/bin/exec"]);
    rig.write_overlay(serde_json::json!({"joinable": with_exec}));
    rig.trust_overlay();
    let report = rig.under_test(true).roster().unwrap();
    let joinable: Vec<(&str, bool)> = report
        .configs
        .iter()
        .map(|e| (e.name.as_str(), e.joinable))
        .collect();
    assert!(joinable.contains(&("joinable", true)), "{joinable:?}");
    assert!(joinable.contains(&("global", false)), "{joinable:?}");
    assert!(report.configs.iter().all(|e| e.problem.is_none()));
}

#[test]
fn the_revision_changes_when_a_layer_or_the_trust_record_is_written_and_is_stable_otherwise() {
    let rig = Rig::new();
    let roster = rig.under_test(true);
    let initial = roster.revision();
    assert_eq!(roster.revision(), initial, "stable without a write");
    // A metadata change (a later mtime or another length) is what the
    // probe sees; sleep past coarse filesystem timestamps.
    std::thread::sleep(std::time::Duration::from_millis(20));
    rig.write_overlay(serde_json::json!({"repo": entry(true, Some("https://repo.test/r"))}));
    let after_overlay = roster.revision();
    assert_ne!(
        after_overlay, initial,
        "an overlay write moves the revision"
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    rig.trust_overlay();
    let after_trust = roster.revision();
    assert_ne!(
        after_trust, after_overlay,
        "a trust record write moves the revision"
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(
        rig.base_dir.join("config.json"),
        serde_json::json!({"container_configs": {"global": entry(true, Some("https://global.test/g2"))}})
            .to_string(),
    )
    .unwrap();
    assert_ne!(
        roster.revision(),
        after_trust,
        "a global write moves the revision"
    );
    // A same-length rewrite that lands within one timestamp tick still
    // moves the token (inode change time / inode), so a cached roster
    // line cannot outlive an atomic replace of a layer.
    let before_rewrite = roster.revision();
    let global = rig.base_dir.join("config.json");
    let same_length = std::fs::read_to_string(&global)
        .unwrap()
        .replace("global.test/g2", "global.test/g3");
    let tmp = rig.base_dir.join("config.json.tmp");
    std::fs::write(&tmp, same_length).unwrap();
    std::fs::rename(&tmp, &global).unwrap();
    assert_ne!(
        roster.revision(),
        before_rewrite,
        "an atomic same-length replace moves the revision"
    );
    // No read of the configuration is needed to answer: an unparseable
    // global file still yields a revision.
    std::fs::write(rig.base_dir.join("config.json"), "{not json").unwrap();
    assert!(!roster.revision().is_empty());
    assert!(roster.roster().is_err());
}
