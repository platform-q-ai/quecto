//! The configuration load/validation error (split out of `config.rs`).

#[derive(Debug)]
pub enum ConfigError {
    Io(String, std::io::Error),
    Parse(serde_json::Error),
    WorkflowStep(String),
    /// Workflow template directory discovery/load failure (slice 2); the
    /// message names the offending file or directory.
    WorkflowTemplate(String),
    /// Unrecognised `agents.defaults.effort` value (#1066).
    InvalidEffort(String),
    /// Invalid `container_configs` default labeling (#1410).
    ContainerConfigs(String),
    /// Invalid `admission` section (#1679).
    Admission(String),
    /// An invalid context budget (#2349 review L3).
    ContextBudget(String),
    /// A key or environment override that was removed (#2414).
    RemovedKey(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(path, err) => {
                write!(f, "failed to read config file '{}': {}", path, err)
            }
            ConfigError::Parse(err) => write!(f, "failed to parse config: {}", err),
            ConfigError::WorkflowStep(err) => write!(f, "failed to load workflow step: {err}"),
            ConfigError::WorkflowTemplate(err) => {
                write!(f, "failed to load workflow template: {err}")
            }
            ConfigError::InvalidEffort(v) => write!(
                f,
                "invalid effort level '{}'; expected one of: {}",
                v,
                crate::domain::provider::EffortLevel::VALID_VALUES
            ),
            ConfigError::ContainerConfigs(err) => {
                write!(f, "invalid container_configs: {err}")
            }
            ConfigError::Admission(err) => write!(f, "invalid admission config: {err}"),
            ConfigError::ContextBudget(err) => write!(f, "invalid context budget: {err}"),
            ConfigError::RemovedKey(err) => write!(f, "removed config key: {err}"),
        }
    }
}

impl std::error::Error for ConfigError {}
