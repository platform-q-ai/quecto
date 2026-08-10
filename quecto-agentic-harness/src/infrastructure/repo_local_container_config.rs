use std::collections::{BTreeSet, HashMap};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::infrastructure::config::{Config, ConfigError, ContainerConfig};

#[derive(Debug, Clone)]
pub struct EffectiveContainerConfigs {
    pub config: Config,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustDecision {
    Approved,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoLocalConfigIdentity {
    pub path: PathBuf,
    pub content_hash: String,
}

pub trait RepoLocalContainerConfigTrust {
    fn decide(&mut self, identity: &RepoLocalConfigIdentity) -> TrustDecision;

    fn record_approved(&mut self, _identity: &RepoLocalConfigIdentity) {}
}

#[derive(Debug, Default)]
pub struct PersistentRepoLocalContainerConfigTrust {
    store_path: Option<PathBuf>,
    prompt_on_miss: bool,
}

impl PersistentRepoLocalContainerConfigTrust {
    pub fn new() -> Self {
        Self {
            store_path: dirs::state_dir()
                .or_else(dirs::home_dir)
                .map(|root| root.join(".quecto/container-config-trust.json")),
            prompt_on_miss: true,
        }
    }

    pub fn read_only() -> Self {
        Self {
            prompt_on_miss: false,
            ..Self::new()
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_store_path(store_path: PathBuf) -> Self {
        Self {
            store_path: Some(store_path),
            prompt_on_miss: true,
        }
    }
}

impl RepoLocalContainerConfigTrust for PersistentRepoLocalContainerConfigTrust {
    fn decide(&mut self, identity: &RepoLocalConfigIdentity) -> TrustDecision {
        if let Some(store_path) = &self.store_path {
            let store = read_store(store_path);
            let path = identity.path.to_string_lossy().to_string();
            if store
                .approved
                .get(&path)
                .is_some_and(|hashes| hashes.contains(&identity.content_hash))
            {
                return TrustDecision::Approved;
            }
        }
        if !self.prompt_on_miss || !prompt_approval(identity) {
            return TrustDecision::Denied;
        }
        TrustDecision::Approved
    }

    fn record_approved(&mut self, identity: &RepoLocalConfigIdentity) {
        let Some(store_path) = &self.store_path else {
            return;
        };
        let mut store = read_store(store_path);
        store
            .approved
            .entry(identity.path.to_string_lossy().to_string())
            .or_default()
            .insert(identity.content_hash.clone());
        let _ = write_store(store_path, &store);
    }
}

pub fn effective_container_configs_for_checkout(
    global: Config,
    checkout: &Path,
    trust: &mut dyn RepoLocalContainerConfigTrust,
) -> Result<EffectiveContainerConfigs, ConfigError> {
    let local_path = checkout.join(".quecto").join("config.json");
    let content = match std::fs::read(&local_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(EffectiveContainerConfigs {
                config: global,
                diagnostics: Vec::new(),
            });
        }
        Err(error) => return Err(ConfigError::Io(local_path.display().to_string(), error)),
    };

    let identity = RepoLocalConfigIdentity {
        path: local_path
            .canonicalize()
            .unwrap_or_else(|_| absolutize(&local_path)),
        content_hash: hex_sha256(&content),
    };

    match trust.decide(&identity) {
        TrustDecision::Denied => Ok(EffectiveContainerConfigs {
            config: global,
            diagnostics: vec![format!(
                "untrusted repo-local container config ignored: {}",
                local_path.display()
            )],
        }),
        TrustDecision::Approved => {
            let local = load_repo_local_container_configs(&content)?;
            trust.record_approved(&identity);
            Ok(EffectiveContainerConfigs {
                config: merge_container_configs(global, local),
                diagnostics: Vec::new(),
            })
        }
    }
}

#[derive(Debug, Deserialize)]
struct RepoLocalContainerOnly {
    #[serde(default)]
    container_configs: HashMap<String, ContainerConfig>,
    #[serde(default)]
    container_scripts: Option<serde_json::Value>,
}

fn load_repo_local_container_configs(
    content: &[u8],
) -> Result<HashMap<String, ContainerConfig>, ConfigError> {
    let local: RepoLocalContainerOnly =
        serde_json::from_slice(content).map_err(ConfigError::Parse)?;
    if local.container_scripts.is_some() {
        return Err(ConfigError::ContainerConfigs(
            "the `container_scripts` key was renamed to `container_configs` (#1410); entries are now a flat map of container configs with exactly one labeled \"default\": true — see docs/container-runtimes.md"
                .into(),
        ));
    }
    validate_container_configs(&local.container_configs)?;
    Ok(local.container_configs)
}

fn validate_container_configs(
    container_configs: &HashMap<String, ContainerConfig>,
) -> Result<(), ConfigError> {
    if container_configs.is_empty() {
        return Ok(());
    }
    let mut names: Vec<&str> = container_configs.keys().map(String::as_str).collect();
    names.sort_unstable();
    let defaults: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| container_configs[*name].default)
        .collect();
    match defaults.as_slice() {
        [_] => Ok(()),
        [] => Err(ConfigError::ContainerConfigs(format!(
            "no container config is labeled \"default\": true (configured: {})",
            names.join(", ")
        ))),
        multiple => Err(ConfigError::ContainerConfigs(format!(
            "multiple container configs are labeled \"default\": true ({}); exactly one is allowed",
            multiple.join(", ")
        ))),
    }
}

fn merge_container_configs(mut global: Config, local: HashMap<String, ContainerConfig>) -> Config {
    if local.values().any(|entry| entry.default) {
        for entry in global.container_configs.values_mut() {
            entry.default = false;
        }
    }
    global.container_configs.extend(local);
    global
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct TrustStore {
    #[serde(default)]
    approved: HashMap<String, BTreeSet<String>>,
}

fn read_store(path: &Path) -> TrustStore {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn write_store(path: &Path, store: &TrustStore) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(store)?)
}

fn prompt_approval(identity: &RepoLocalConfigIdentity) -> bool {
    if !io::stdin().is_terminal() {
        eprintln!(
            "repo-local container_configs from {} are untrusted (sha256 {}) and were ignored; approve from an interactive terminal before use",
            identity.path.display(),
            identity.content_hash
        );
        return false;
    }
    eprintln!(
        "Trust repo-local container_configs from {} (sha256 {})? [y/N] ",
        identity.path.display(),
        identity.content_hash
    );
    let _ = io::stderr().flush();
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map(|_| matches!(input.trim(), "y" | "Y" | "yes" | "YES" | "Yes"))
        .unwrap_or(false)
}

fn absolutize(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

fn hex_sha256(content: &[u8]) -> String {
    let digest = Sha256::digest(content);
    format!("{digest:x}")
}

#[cfg(test)]
#[path = "repo_local_container_config_tests.rs"]
mod tests;
