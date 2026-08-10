use super::*;
use crate::infrastructure::config::{Config, ContainerConfig};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn config_with(name: &str, default: bool, create: &str) -> Config {
    let mut container_configs = HashMap::new();
    container_configs.insert(
        name.to_string(),
        ContainerConfig {
            default,
            create: vec![create.into()],
            cleanup: vec!["/bin/true".into()],
            exec: vec![],
            kill: vec![],
            inspect: vec![],
        },
    );
    Config {
        container_configs,
        ..Default::default()
    }
}

fn write_repo_local_config(checkout: &Path, name: &str, default: bool, create: &str) {
    let dir = checkout.join(".quecto");
    std::fs::create_dir_all(&dir).unwrap();
    let config = serde_json::json!({
        "container_configs": {
            name: {"default": default, "create": [create], "cleanup": ["/bin/true"]}
        }
    });
    std::fs::write(
        dir.join("config.json"),
        serde_json::to_string_pretty(&config).unwrap(),
    )
    .unwrap();
}

#[derive(Default)]
struct RecordingTrust {
    decisions: Vec<TrustDecision>,
    seen: Vec<RepoLocalConfigIdentity>,
}

impl RecordingTrust {
    fn approving() -> Self {
        Self {
            decisions: vec![TrustDecision::Approved],
            seen: vec![],
        }
    }

    fn denying() -> Self {
        Self {
            decisions: vec![TrustDecision::Denied],
            seen: vec![],
        }
    }
}

impl RepoLocalContainerConfigTrust for RecordingTrust {
    fn decide(&mut self, identity: &RepoLocalConfigIdentity) -> TrustDecision {
        self.seen.push(identity.clone());
        self.decisions.remove(0)
    }

    fn record_approved(&mut self, identity: &RepoLocalConfigIdentity) {
        self.seen.push(identity.clone());
    }
}

#[test]
fn effective_container_configs_untrusted_repo_local_ignored_visibly() {
    let checkout = TempDir::new().unwrap();
    write_repo_local_config(checkout.path(), "local-default", true, "/tmp/local-create");
    let global = config_with("global-default", true, "/tmp/global-create");
    let mut trust = RecordingTrust::denying();

    let effective =
        effective_container_configs_for_checkout(global, checkout.path(), &mut trust).unwrap();

    assert!(
        !effective
            .config
            .container_configs
            .contains_key("local-default"),
        "untrusted repo-local config must not be selectable"
    );
    assert!(
        effective
            .config
            .container_configs
            .contains_key("global-default")
    );
    let diagnostics = effective.diagnostics.join("\n");
    assert!(diagnostics.contains("untrusted"), "{diagnostics}");
    assert!(diagnostics.contains("ignored"), "{diagnostics}");
    assert!(diagnostics.contains(".quecto/config.json"), "{diagnostics}");
}

#[test]
fn effective_container_configs_first_sighting_approval_adds_local_and_records_identity() {
    let checkout = TempDir::new().unwrap();
    write_repo_local_config(checkout.path(), "local-default", true, "/tmp/local-create");
    let global = config_with("global-default", true, "/tmp/global-create");
    let mut trust = RecordingTrust::approving();

    let effective =
        effective_container_configs_for_checkout(global, checkout.path(), &mut trust).unwrap();

    assert!(
        effective
            .config
            .container_configs
            .contains_key("local-default")
    );
    assert!(!trust.seen.is_empty(), "first sighting must ask trust gate");
    assert!(trust.seen[0].path.ends_with(".quecto/config.json"));
    assert!(!trust.seen[0].content_hash.is_empty());
}

#[test]
fn effective_container_configs_trusted_local_default_wins_after_per_source_validation() {
    let checkout = TempDir::new().unwrap();
    write_repo_local_config(checkout.path(), "local-default", true, "/tmp/local-create");
    let global = config_with("global-default", true, "/tmp/global-create");
    let mut trust = RecordingTrust::approving();

    let effective =
        effective_container_configs_for_checkout(global, checkout.path(), &mut trust).unwrap();
    let defaults: Vec<_> = effective
        .config
        .container_configs
        .iter()
        .filter(|(_, c)| c.default)
        .map(|(name, _)| name.as_str())
        .collect();

    assert_eq!(defaults, vec!["local-default"]);
}

#[test]
fn effective_container_configs_trusted_local_shadows_global_name() {
    let checkout = TempDir::new().unwrap();
    write_repo_local_config(checkout.path(), "sandbox", true, "/tmp/local-create");
    let global = config_with("sandbox", true, "/tmp/global-create");
    let mut trust = RecordingTrust::approving();

    let effective =
        effective_container_configs_for_checkout(global, checkout.path(), &mut trust).unwrap();

    assert_eq!(
        effective.config.container_configs["sandbox"].create,
        vec!["/tmp/local-create".to_string()]
    );
}

#[test]
fn effective_container_configs_repo_local_legacy_key_uses_existing_rename_guidance() {
    let checkout = TempDir::new().unwrap();
    let dir = checkout.path().join(".quecto");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("config.json"),
        r#"{"container_scripts":{"default":{"create":["/tmp/bad"]}}}"#,
    )
    .unwrap();
    let global = config_with("global-default", true, "/tmp/global-create");
    let mut trust = RecordingTrust::approving();

    let err = effective_container_configs_for_checkout(global, checkout.path(), &mut trust)
        .unwrap_err()
        .to_string();

    assert!(err.contains("container_scripts"), "{err}");
    assert!(err.contains("container_configs"), "{err}");
    assert!(err.contains("docs/container-runtimes.md"), "{err}");
}

#[test]
fn effective_container_configs_changed_content_requires_fresh_trust() {
    let checkout = TempDir::new().unwrap();
    write_repo_local_config(checkout.path(), "local-default", true, "/tmp/safe");
    let global = config_with("global-default", true, "/tmp/global-create");
    let mut trust = RecordingTrust::approving();

    let first =
        effective_container_configs_for_checkout(global.clone(), checkout.path(), &mut trust)
            .unwrap();
    assert_eq!(
        first.config.container_configs["local-default"].create,
        vec!["/tmp/safe".to_string()]
    );

    write_repo_local_config(checkout.path(), "local-default", true, "/tmp/changed");
    let mut denial = RecordingTrust::denying();
    let second =
        effective_container_configs_for_checkout(global, checkout.path(), &mut denial).unwrap();
    assert!(
        !second
            .config
            .container_configs
            .contains_key("local-default")
    );
}

#[test]
fn denied_legacy_repo_local_config_is_ignored_without_blocking_global() {
    let checkout = TempDir::new().unwrap();
    let dir = checkout.path().join(".quecto");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("config.json"),
        r#"{"container_scripts":{"default":{"create":["/tmp/bad"]}}}"#,
    )
    .unwrap();
    let global = config_with("global-default", true, "/tmp/global-create");
    let mut trust = RecordingTrust::denying();

    let effective = effective_container_configs_for_checkout(global, checkout.path(), &mut trust)
        .expect("denied untrusted legacy file must not block global configs");

    assert!(
        effective
            .config
            .container_configs
            .contains_key("global-default")
    );
    assert_eq!(
        trust.seen.len(),
        1,
        "denied content should not be persisted"
    );
}

#[test]
fn missing_repo_local_config_leaves_global_without_diagnostics() {
    let checkout = TempDir::new().unwrap();
    let global = config_with("global-default", true, "/tmp/global-create");
    let mut trust = RecordingTrust::denying();

    let effective = effective_container_configs_for_checkout(global, checkout.path(), &mut trust)
        .expect("missing repo-local config should be ignored");

    assert!(effective.diagnostics.is_empty());
    assert!(
        effective
            .config
            .container_configs
            .contains_key("global-default")
    );
    assert!(trust.seen.is_empty());
}

#[test]
fn approved_repo_local_without_default_is_rejected_before_merge() {
    let checkout = TempDir::new().unwrap();
    let dir = checkout.path().join(".quecto");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("config.json"),
        r#"{"container_configs":{"local":{"create":["/tmp/local"],"cleanup":["/bin/true"]}}}"#,
    )
    .unwrap();
    let global = config_with("global-default", true, "/tmp/global-create");
    let mut trust = RecordingTrust::approving();

    let err = effective_container_configs_for_checkout(global, checkout.path(), &mut trust)
        .unwrap_err()
        .to_string();

    assert!(err.contains("no container config"), "{err}");
}

#[test]
fn approved_repo_local_with_multiple_defaults_is_rejected_before_merge() {
    let checkout = TempDir::new().unwrap();
    let dir = checkout.path().join(".quecto");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("config.json"),
        r#"{"container_configs":{"a":{"default":true,"create":["/tmp/a"],"cleanup":["/bin/true"]},"b":{"default":true,"create":["/tmp/b"],"cleanup":["/bin/true"]}}}"#,
    )
    .unwrap();
    let global = config_with("global-default", true, "/tmp/global-create");
    let mut trust = RecordingTrust::approving();

    let err = effective_container_configs_for_checkout(global, checkout.path(), &mut trust)
        .unwrap_err()
        .to_string();

    assert!(err.contains("multiple container configs"), "{err}");
}

#[test]
fn approved_repo_local_empty_container_map_is_noop() {
    let checkout = TempDir::new().unwrap();
    let dir = checkout.path().join(".quecto");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.json"), r#"{"container_configs":{}}"#).unwrap();
    let global = config_with("global-default", true, "/tmp/global-create");
    let mut trust = RecordingTrust::approving();

    let effective = effective_container_configs_for_checkout(global, checkout.path(), &mut trust)
        .expect("empty repo-local container_configs should be a no-op overlay");

    assert!(
        effective
            .config
            .container_configs
            .contains_key("global-default")
    );
}

#[test]
fn persistent_trust_persists_hash_and_read_only_denies_unknown_hash() {
    let store_dir = TempDir::new().unwrap();
    let store_path = store_dir.path().join("trust.json");
    let identity = RepoLocalConfigIdentity {
        path: store_dir.path().join("repo/.quecto/config.json"),
        content_hash: "abc123".into(),
    };
    let changed = RepoLocalConfigIdentity {
        content_hash: "def456".into(),
        ..identity.clone()
    };

    let mut trust = PersistentRepoLocalContainerConfigTrust::with_store_path(store_path.clone());
    trust.record_approved(&identity);
    assert_eq!(trust.decide(&identity), TrustDecision::Approved);
    assert_eq!(trust.decide(&changed), TrustDecision::Denied);

    let content = std::fs::read_to_string(store_path).unwrap();
    assert!(content.contains("abc123"), "{content}");
    assert!(!content.contains("def456"), "{content}");
}

#[test]
fn persistent_trust_read_only_constructor_denies_unknown_without_prompt() {
    let identity = RepoLocalConfigIdentity {
        path: TempDir::new()
            .unwrap()
            .path()
            .join("repo/.quecto/config.json"),
        content_hash: "unknown".into(),
    };

    let mut trust = PersistentRepoLocalContainerConfigTrust::read_only();

    assert_eq!(trust.decide(&identity), TrustDecision::Denied);
}

#[test]
fn trust_store_invalid_json_is_treated_as_empty_and_parentless_write_succeeds() {
    let store_dir = TempDir::new().unwrap();
    let invalid_store = store_dir.path().join("trust.json");
    std::fs::write(&invalid_store, "not json").unwrap();
    assert!(read_store(&invalid_store).approved.is_empty());

    let cwd = std::env::current_dir().unwrap();
    std::env::set_current_dir(store_dir.path()).unwrap();
    let result = write_store(Path::new("trust-parentless.json"), &TrustStore::default());
    std::env::set_current_dir(cwd).unwrap();

    result.unwrap();
    assert!(store_dir.path().join("trust-parentless.json").exists());
}

#[test]
fn prompt_approval_non_interactive_denies_untrusted_identity() {
    let identity = RepoLocalConfigIdentity {
        path: TempDir::new()
            .unwrap()
            .path()
            .join("repo/.quecto/config.json"),
        content_hash: "abc123".into(),
    };

    assert!(!prompt_approval(&identity));
}

#[test]
fn absolutize_covers_absolute_and_relative_paths() {
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(
        absolutize(Path::new("relative/config.json")),
        cwd.join("relative/config.json")
    );
    let absolute = cwd.join("absolute/config.json");
    assert_eq!(absolutize(&absolute), absolute);
}

#[test]
fn default_trust_record_approved_is_noop_and_derived_surfaces_are_used() {
    struct DenyOnly;
    impl RepoLocalContainerConfigTrust for DenyOnly {
        fn decide(&mut self, _identity: &RepoLocalConfigIdentity) -> TrustDecision {
            TrustDecision::Denied
        }
    }

    let identity = RepoLocalConfigIdentity {
        path: PathBuf::from("/tmp/repo/.quecto/config.json"),
        content_hash: "abc123".into(),
    };
    let mut trust = DenyOnly;

    trust.record_approved(&identity);
    assert_eq!(trust.decide(&identity), TrustDecision::Denied);
    assert_eq!(TrustDecision::Approved.clone(), TrustDecision::Approved);
    assert_eq!(identity.clone(), identity);
    assert!(format!("{:?}", identity).contains("abc123"));

    let effective = EffectiveContainerConfigs {
        config: Config::default(),
        diagnostics: vec!["untrusted".into()],
    };
    let cloned = effective.clone();
    assert!(format!("{:?}", cloned).contains("untrusted"));

    let store = TrustStore::default();
    assert!(format!("{:?}", store).contains("approved"));
    assert!(
        format!("{:?}", PersistentRepoLocalContainerConfigTrust::default())
            .contains("prompt_on_miss")
    );
}
