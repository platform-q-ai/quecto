use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::workflow::WorkflowConfig;
use crate::infrastructure::persistence::file::json_failure as json;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub agents: AgentConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub workflow: WorkflowConfig,
    /// The switchable event log (#2150).
    #[serde(default)]
    pub telemetry: telemetry::TelemetryConfig,
    /// Named container configs (#1410): each entry is a complete,
    /// self-contained definition of a container working context (repository,
    /// auth, and runtime mechanics all live in the config's own argv).
    /// Exactly one entry must carry `"default": true` when any are defined —
    /// validated at load time, not at spawn time.
    #[serde(default)]
    pub container_configs: HashMap<String, ContainerConfig>,
    /// Shared inference admission (#1679). Absent keeps admission disabled.
    #[serde(default)]
    pub admission: Option<self::config_admission::AdmissionSection>,
    /// The UDS extensions every agent launches (#2446); global-only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<extensions::ExtensionConfig>,
    /// The board server's TCP addresses (epic #2705 B5); global-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_server: Option<board_server::BoardServerConfig>,
    /// Not serialized: base directory for the default authority directory.
    #[serde(skip)]
    pub(super) admission_base_dir: PathBuf,
}

// #2403: moved to its own module for the line cap.
mod container_config;
pub use container_config::ContainerConfig;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentConfig {
    #[serde(default)]
    pub defaults: AgentDefaults,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefaults {
    #[serde(default = "default_workspace")]
    pub workspace: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default = "default_max_tool_iterations")]
    pub max_tool_iterations: u32,
    /// Deprecated compatibility key. Filesystem workspace restriction was removed;
    /// old configs may still contain this key, but it is ignored.
    #[serde(default, rename = "restrict_to_workspace", skip_serializing)]
    pub _deprecated_restrict_to_workspace: Option<bool>,
    #[serde(default = "default_exec_max_capture_bytes")]
    pub exec_max_capture_bytes: usize,
    #[serde(default = "default_max_session_messages")]
    pub max_session_messages: usize,
    /// The emergency ladder's ceiling (#1044); the watermark marks scale
    /// down under it (#2401).
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
    /// The ceiling once this process takes part in a swarm (#2342): the
    /// lower of it and `max_context_tokens` applies from then on.
    #[serde(default = "default_swarm_max_context_tokens")]
    pub swarm_max_context_tokens: usize,
    /// The watermark marks (#2403), in `config/context_marks.rs`.
    #[serde(flatten)]
    pub context_marks: context_marks::ContextMarksConfig,
    /// The removed context keys (#2414), refused at load.
    #[serde(flatten)]
    pub removed_context_keys: removed_keys::RemovedContextKeys,
    /// How many most-recent turns the emergency ladder tail-pins (#1045).
    #[serde(default = "default_pin_recent_turns")]
    pub pin_recent_turns: u32,
    /// Effort level for 4.6 models (`low`/`medium`/`high`/`max`).
    /// Defaults to `None`; provider applies `low` for 4.6 models when unset.
    #[serde(default)]
    pub effort: Option<String>,
    /// Deprecated compatibility key. The per-command allowlist was removed in
    /// #1620; command policy is denylist-only. Old configs may still contain
    /// this key, but it is ignored (a warning is logged at startup). It is
    /// preserved on save so a round-trip through the loader does not silently
    /// delete an operator's setting.
    #[serde(
        default,
        rename = "command_allowlist",
        skip_serializing_if = "Option::is_none"
    )]
    pub _deprecated_command_allowlist: Option<Vec<String>>,
}

impl Default for AgentDefaults {
    fn default() -> Self {
        Self {
            workspace: default_workspace(),
            model: default_model(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            max_tool_iterations: default_max_tool_iterations(),
            _deprecated_restrict_to_workspace: None,
            exec_max_capture_bytes: default_exec_max_capture_bytes(),
            max_session_messages: default_max_session_messages(),
            max_context_tokens: default_max_context_tokens(),
            swarm_max_context_tokens: default_swarm_max_context_tokens(),
            context_marks: Default::default(),
            removed_context_keys: Default::default(),
            pin_recent_turns: default_pin_recent_turns(),
            effort: None,
            _deprecated_command_allowlist: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProvidersConfig {
    #[serde(default)]
    pub openai: ProviderEntry,
    #[serde(default)]
    pub anthropic: ProviderEntry,
    #[serde(default)]
    pub openai_compatible: OpenAiCompatibleConfig,
    /// How long, in seconds, a model a provider refused for the account is
    /// held unavailable (#2435); a reply the provider serves for it releases
    /// it sooner. Absent: [`DEFAULT_MODEL_REFUSAL_TTL_SECS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_refusal_ttl_secs: Option<u64>,
}

/// The default hold of a refused model (#2435): an hour, so a transient
/// refusal (a rollout's 404) never bans a model for long.
pub const DEFAULT_MODEL_REFUSAL_TTL_SECS: u64 = 3600;

impl ProvidersConfig {
    /// How long a model a provider refused is held unavailable; `0` holds
    /// none.
    pub fn model_refusal_ttl(&self) -> std::time::Duration {
        std::time::Duration::from_secs(
            self.model_refusal_ttl_secs
                .unwrap_or(DEFAULT_MODEL_REFUSAL_TTL_SECS),
        )
    }
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ProviderEntry {
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub api_base: String,
    /// When true for the `openai` slot, use the config `api_key` directly and
    /// do not convert ChatGPT OAuth JWTs from the credential store into Codex.
    #[serde(default)]
    pub disable_codex_routing: bool,
    /// Its stream limits (#2433), flat in the entry.
    #[serde(flatten)]
    pub stream_limits: StreamLimitsConfig,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct OpenAiCompatibleConfig {
    #[serde(default)]
    pub endpoints: Vec<OpenAiCompatibleEndpoint>,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct OpenAiCompatibleEndpoint {
    #[serde(default)]
    pub prefix: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub api_base: String,
    #[serde(default)]
    pub allow_remote_http: bool,
    /// Its stream limits (#2433), flat in the entry.
    #[serde(flatten)]
    pub stream_limits: StreamLimitsConfig,
}

impl std::fmt::Debug for ProviderEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderEntry")
            .field("api_key", &"[REDACTED]")
            .field("api_base", &self.api_base)
            .field("disable_codex_routing", &self.disable_codex_routing)
            .field("stream_limits", &self.stream_limits)
            .finish()
    }
}

impl std::fmt::Debug for OpenAiCompatibleConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatibleConfig")
            .field("endpoints", &self.endpoints)
            .finish()
    }
}

impl std::fmt::Debug for OpenAiCompatibleEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiCompatibleEndpoint")
            .field("prefix", &self.prefix)
            .field("api_key", &"[REDACTED]")
            .field("api_base", &self.api_base)
            .field("allow_remote_http", &self.allow_remote_http)
            .field("stream_limits", &self.stream_limits)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolsConfig {
    #[serde(default)]
    pub web: WebToolConfig,
    /// Durable catalogue-backed user policy preferences, keyed by stable tool id.
    #[serde(default)]
    pub policy: ToolPolicyConfig,
    /// The grep tool's relevance ranking and search log (#2136).
    #[serde(default)]
    pub grep: GrepToolConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolPolicyConfig {
    /// Stable tool id -> persisted profile availability scope. Unknown/removed
    /// ids are preserved on load/save and ignored until a matching tool exists.
    #[serde(default)]
    pub entries: HashMap<String, ToolPolicyEntryConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolPolicyEntryConfig {
    pub scope: crate::domain::tool_policy::value_objects::tool_descriptor::ProfileAvailabilityScope,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebToolConfig {
    #[serde(default)]
    pub brave: BraveConfig,
    #[serde(default)]
    pub duckduckgo: DuckDuckGoConfig,
    #[serde(default)]
    pub fetch: FetchConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FetchConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Maximum response size in KB returned to the LLM. Default: 32.
    #[serde(default = "default_max_response_kb")]
    pub max_response_kb: u32,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_response_kb: default_max_response_kb(),
        }
    }
}

fn default_max_response_kb() -> u32 {
    32
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct BraveConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_max_results")]
    pub max_results: u32,
}

impl std::fmt::Debug for BraveConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BraveConfig")
            .field("enabled", &self.enabled)
            .field("api_key", &"[REDACTED]")
            .field("max_results", &self.max_results)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DuckDuckGoConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_max_results")]
    pub max_results: u32,
}

fn default_workspace() -> String {
    "~/.quecto/workspace".to_string()
}
fn default_model() -> String {
    "gpt-6.1-sol".to_string()
}
fn default_max_tokens() -> u32 {
    8192
}
fn default_temperature() -> f32 {
    0.7
}
fn default_max_tool_iterations() -> u32 {
    999_999
}
fn default_exec_max_capture_bytes() -> usize {
    1024 * 1024
}
fn default_max_session_messages() -> usize {
    200
}
fn default_pin_recent_turns() -> u32 {
    2
}
fn default_max_context_tokens() -> usize {
    // Application-level context ceiling (owner decision 2026-10-01, epic
    // #2401): 300k, so the 256k/70k watermark marks apply unscaled. It is
    // still lowered to what the model's window leaves the prompt beside the
    // reply (#2405): 258,400 on the Codex surface.
    300_000
}
fn default_swarm_max_context_tokens() -> usize {
    // A swarm member's context ceiling. #2342 kept members lean at 48k; the
    // owner raised every agent, members included, to 300k (2026-10-01, epic
    // #2401), so long-running members use the 256k/70k watermark marks.
    // Lower it in config for a lean member: the marks scale down with it.
    300_000
}
fn default_max_results() -> u32 {
    5
}
impl Config {
    /// Load config from a JSON file. A missing file yields the default config
    /// (quecto is zero-config: every field has a sensible default, and
    /// credentials come from env vars or `quecto auth login`). Other IO errors
    /// and malformed JSON still fail.
    pub fn load(path: &str) -> Result<Self, ConfigError> {
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(ConfigError::Io(path.to_string(), e)),
        };
        let mut value: serde_json::Value = json::from_str(&content).map_err(ConfigError::Parse)?;
        Self::reject_retired_keys(&value)?;
        let resolved_references = resolve_workflow_step_entries(&mut value, Path::new(path))?;
        // Deserializing the resolved Value loses line/column error context, so
        // only pay that cost when a reference was actually substituted.
        if resolved_references {
            Self::from_document(value)
        } else {
            let config: Config = json::from_str(&content).map_err(ConfigError::Parse)?;
            config.validated()
        }
    }

    /// Reject an unrecognised `agents.defaults.effort` at configuration time
    /// with an error naming every valid value (#1066).
    pub(super) fn validate_effort(&self) -> Result<(), ConfigError> {
        if let Some(effort) = self.agents.defaults.effort.as_deref()
            && crate::domain::inference::value_objects::provider::EffortLevel::parse(effort)
                .is_none()
        {
            return Err(ConfigError::InvalidEffort(effort.to_string()));
        }
        Ok(())
    }

    /// Exactly one container config must be labeled `"default": true` when
    /// any are defined (#1410). Zero or multiple defaults fail here, at load
    /// time, naming the configured entries so the caller can fix the file.
    pub(super) fn validate_container_configs(&self) -> Result<(), ConfigError> {
        if self.container_configs.is_empty() {
            return Ok(());
        }
        let mut names: Vec<&str> = self.container_configs.keys().map(String::as_str).collect();
        names.sort_unstable();
        let defaults: Vec<&str> = names
            .iter()
            .copied()
            .filter(|name| self.container_configs[*name].default)
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

    /// Test-only seam for the load-time default-label validation (#1410).
    #[cfg(any(test, feature = "test-support"))]
    pub fn validate_container_configs_for_test(&self) -> Result<(), ConfigError> {
        self.validate_container_configs()
    }

    /// Load config from a JSON file, then apply environment variable overrides.
    ///
    /// Environment variable naming convention: `QUECTO_AGENTS_DEFAULTS_MODEL`
    /// maps to `config.agents.defaults.model`.
    pub fn load_with_env(
        path: &str,
        env_overrides: &HashMap<String, String>,
    ) -> Result<Self, ConfigError> {
        Self::load(path)?.with_env_overrides(env_overrides)
    }

    /// Apply environment variable overrides to a mutable config.
    ///
    /// Supported keys:
    /// - `QUECTO_AGENTS_DEFAULTS_MODEL` → agents.defaults.model
    /// - `QUECTO_AGENTS_DEFAULTS_MAX_TOKENS` → agents.defaults.max_tokens
    /// - `QUECTO_AGENTS_DEFAULTS_TEMPERATURE` → agents.defaults.temperature
    /// - `QUECTO_AGENTS_DEFAULTS_WORKSPACE` → agents.defaults.workspace
    /// - `QUECTO_AGENTS_DEFAULTS_MAX_SESSION_MESSAGES` → agents.defaults.max_session_messages
    /// - `QUECTO_MAX_CONTEXT_TOKENS` → agents.defaults.max_context_tokens
    /// - `QUECTO_SWARM_MAX_CONTEXT_TOKENS` → agents.defaults.swarm_max_context_tokens
    /// - `QUECTO_CONTEXT_{HIGH,LOW}_TOKENS` → the watermark marks (#2403)
    /// - the removed context overrides are refused (#2414)
    /// - `QUECTO_AGENTS_DEFAULTS_EFFORT` → agents.defaults.effort
    /// - `OPENAI_API_KEY` → providers.openai.api_key
    /// - `ANTHROPIC_API_KEY` → providers.anthropic.api_key
    pub(super) fn apply_env_overrides(config: &mut Config, env: &HashMap<String, String>) {
        if let Some(v) = env.get("QUECTO_AGENTS_DEFAULTS_MODEL") {
            config.agents.defaults.model = v.clone();
        }
        if let Some(v) = env.get("QUECTO_AGENTS_DEFAULTS_MAX_TOKENS")
            && let Ok(n) = v.parse::<u32>()
        {
            config.agents.defaults.max_tokens = n;
        }
        if let Some(v) = env.get("QUECTO_AGENTS_DEFAULTS_TEMPERATURE")
            && let Ok(f) = v.parse::<f32>()
        {
            config.agents.defaults.temperature = f;
        }
        if let Some(v) = env.get("QUECTO_AGENTS_DEFAULTS_WORKSPACE") {
            config.agents.defaults.workspace = v.clone();
        }
        if let Some(v) = env.get("QUECTO_AGENTS_DEFAULTS_MAX_SESSION_MESSAGES")
            && let Ok(n) = v.parse::<usize>()
        {
            config.agents.defaults.max_session_messages = n;
        }
        if let Some(v) = env.get("QUECTO_MAX_CONTEXT_TOKENS")
            && let Ok(n) = v.parse::<usize>()
        {
            config.agents.defaults.max_context_tokens = n;
        }
        if let Some(v) = env.get("QUECTO_SWARM_MAX_CONTEXT_TOKENS")
            && let Ok(n) = v.parse::<usize>()
        {
            config.agents.defaults.swarm_max_context_tokens = n;
        }
        context_marks::apply_env_overrides(&mut config.agents.defaults, env);
        removed_keys::apply_env_overrides(&mut config.agents.defaults, env);
        if let Some(v) = env.get("OPENAI_API_KEY") {
            config.providers.openai.api_key = v.clone();
        }
        if let Some(v) = env.get("ANTHROPIC_API_KEY") {
            config.providers.anthropic.api_key = v.clone();
        }
        if let Some(v) = env.get("QUECTO_AGENTS_DEFAULTS_EFFORT") {
            // Applied verbatim; `load_with_env` validates afterwards so an
            // unknown value is rejected with an error naming the valid
            // values rather than silently ignored (#1066).
            config.agents.defaults.effort = Some(v.clone());
        }
        if let Some(v) = env.get("QUECTO_TOOLS_WEB_BRAVE_API_KEY") {
            config.tools.web.brave.api_key = v.clone();
        }
    }

    /// Resolve the workspace path, expanding `~` to the user's home directory.
    pub fn workspace_path(&self) -> String {
        crate::infrastructure::tools::native::path_utils::expand_tilde(
            &self.agents.defaults.workspace,
        )
        .to_string_lossy()
        .to_string()
    }
}

// Workflow template directory discovery (slice 2) lives beside this module;
// it reuses the private step-reference resolver below.
#[path = "config_discovery.rs"]
mod discovery;

// #2433: a provider's stream limits, in their own module for the line cap.
mod stream_limits;
pub use discovery::{
    WorkflowTemplateDiscovery, discover_workflow_templates, load_workflow_templates_from_dir,
};
pub use stream_limits::StreamLimitsConfig;

// Workflow step references (slice 1): resolved here, and by the template
// directory discovery above (#2617 moved them out for the line cap).
mod workflow_steps;
#[cfg(test)]
use workflow_steps::{MAX_WORKFLOW_STEP_FILE_BYTES, load_workflow_step};
use workflow_steps::{resolve_workflow_step_entries, resolve_workflow_step_entry};

#[path = "config_error.rs"]
mod error;
pub use error::ConfigError;
pub mod board_server;
pub mod container_config_lookup;
pub mod container_config_roster;
pub mod container_configs;
pub mod extensions;
pub mod grep_tool;
pub mod telemetry;
pub use grep_tool::GrepToolConfig;
pub mod loaders;
pub mod main_worktree;
pub mod mapping;
// #2403: the watermark marks.
pub mod context_marks;
pub mod persistence;
// #2414: the removed context keys, refused at load.
pub mod agents_instructions;
#[cfg(test)]
mod agents_instructions_tests;
pub mod config_admission;
pub mod parent_playbook;
#[cfg(test)]
mod parent_playbook_tests;
pub mod reload;
pub mod removed_keys;
pub mod runtime_configuration;
pub mod writer;

#[cfg(test)]
#[path = "config_admission_tests.rs"]
mod admission_tests;
#[cfg(test)]
#[path = "config_effort_1066_tests.rs"]
mod effort_1066_tests;

#[cfg(test)]
#[path = "config_1495_tests.rs"]
mod config_1495_tests;
#[cfg(test)]
#[path = "config_container_slice2_tests.rs"]
mod container_slice2_tests;
#[cfg(test)]
#[path = "config_cov_tests.rs"]
mod cov_tests;
#[cfg(test)]
#[path = "config_swarm_context_tests.rs"]
mod swarm_context_tests;
#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
