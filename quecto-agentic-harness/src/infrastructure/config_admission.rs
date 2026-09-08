//! Optional `admission` configuration section (#1679 P3): explicit quota
//! groups, opaque aliases, provider-slot bindings and the authority directory.
//! Absent means disabled; present is validated at load time.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::config::{Config, ConfigError};
use crate::domain::inference_admission::{AdmissionConfig, GroupId, GroupPolicy};
use crate::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionGroupSection {
    pub capacity: usize,
    pub reserve: usize,
    pub min_interval_ms: u64,
    pub queue_capacity: usize,
    pub queue_timeout_ms: u64,
    pub attempt_timeout_ms: u64,
    pub fallback_base_ms: u64,
    pub max_cooldown_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionSection {
    /// Private owner-only authority directory. Defaults to `<base_dir>/admission`.
    #[serde(default)]
    pub directory: Option<PathBuf>,
    pub groups: BTreeMap<String, AdmissionGroupSection>,
    /// Opaque endpoint/account alias -> group.
    pub aliases: BTreeMap<String, String>,
    /// Router provider slot -> alias.
    pub bindings: BTreeMap<String, String>,
    #[serde(default = "default_max_scopes")]
    pub max_scopes: usize,
    #[serde(default = "default_terminal_capacity")]
    pub terminal_capacity: usize,
}

fn default_max_scopes() -> usize {
    1024
}
fn default_terminal_capacity() -> usize {
    4096
}

impl AdmissionSection {
    fn proposal(&self) -> Result<AdmissionRuntimeProposal, ConfigError> {
        let invalid = |detail: String| ConfigError::Admission(detail);
        let mut groups = BTreeMap::new();
        for (name, g) in &self.groups {
            groups.insert(
                GroupId::new(name).map_err(|e| invalid(format!("group '{name}': {e:?}")))?,
                GroupPolicy {
                    capacity: g.capacity,
                    reserve: g.reserve,
                    min_interval_ms: g.min_interval_ms,
                    queue_capacity: g.queue_capacity,
                    queue_timeout_ms: g.queue_timeout_ms,
                    attempt_timeout_ms: g.attempt_timeout_ms,
                    fallback_base_ms: g.fallback_base_ms,
                    max_cooldown_ms: g.max_cooldown_ms,
                },
            );
        }
        let mut aliases = BTreeMap::new();
        for (alias, group) in &self.aliases {
            aliases.insert(
                alias.clone(),
                GroupId::new(group).map_err(|e| invalid(format!("alias '{alias}': {e:?}")))?,
            );
        }
        let policy = AdmissionConfig {
            groups,
            aliases,
            max_scopes: self.max_scopes,
            terminal_capacity: self.terminal_capacity,
        };
        policy.validate().map_err(|e| {
            invalid(format!(
                "policy rejected ({e:?}): every group needs capacity>0, reserve<capacity, positive intervals/deadlines, fallback_base<=max_cooldown, and at least one alias"
            ))
        })?;
        for (slot, alias) in &self.bindings {
            if slot.is_empty() || slot.trim() != slot || slot.contains('/') {
                return Err(invalid(format!(
                    "binding slot '{slot}' is not a provider slot name"
                )));
            }
            if !policy.aliases.contains_key(alias) {
                return Err(invalid(format!(
                    "binding '{slot}' names unknown alias '{alias}'"
                )));
            }
        }
        if self.bindings.is_empty() {
            return Err(invalid("at least one provider binding is required".into()));
        }
        Ok(AdmissionRuntimeProposal {
            policy,
            bindings: self.bindings.clone(),
        })
    }
}

impl Config {
    pub(super) fn validate_admission(&self) -> Result<(), ConfigError> {
        self.admission_proposal().map(|_| ())
    }

    /// The validated admission proposal and authority directory, `Ok(None)`
    /// when admission is not configured (normal disabled runtime), or the
    /// configuration error for an invalid section.
    pub fn admission_proposal(
        &self,
    ) -> Result<Option<(PathBuf, AdmissionRuntimeProposal)>, ConfigError> {
        let Some(section) = self.admission.as_ref() else {
            return Ok(None);
        };
        let proposal = section.proposal()?;
        if let Some(directory) = &section.directory {
            if !directory.is_absolute() {
                return Err(ConfigError::Admission(format!(
                    "directory {} must be absolute so every session and the authority resolve the same path",
                    directory.display()
                )));
            }
        }
        let directory = section
            .directory
            .clone()
            .unwrap_or_else(|| default_admission_directory(&self.admission_base_dir));
        // The base directory is identity-mounted into containers by the bundled
        // adapter; an authority at or above it would expose its journal, admin
        // socket and owner token to every child.
        if !self.admission_base_dir.as_os_str().is_empty()
            && self.admission_base_dir.starts_with(&directory)
        {
            return Err(ConfigError::Admission(format!(
                "directory {} is the base directory or one of its ancestors; use a subdirectory such as {}",
                directory.display(),
                default_admission_directory(&self.admission_base_dir).display()
            )));
        }
        Ok(Some((directory, proposal)))
    }

    /// Record the base directory used to resolve the default authority
    /// directory when the section omits `directory`.
    pub fn with_admission_base_dir(mut self, base_dir: &Path) -> Self {
        self.admission_base_dir = base_dir.to_path_buf();
        self
    }
}

pub fn default_admission_directory(base_dir: &Path) -> PathBuf {
    let base = if base_dir.as_os_str().is_empty() {
        crate::infrastructure::tools::path_utils::home_dir()
            .map(|home| home.join(".quecto"))
            .unwrap_or_else(|| PathBuf::from(".quecto"))
    } else {
        base_dir.to_path_buf()
    };
    base.join("admission")
}
