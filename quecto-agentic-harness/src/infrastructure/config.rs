use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::domain::workflow::WorkflowConfig;

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
}

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
    #[serde(default = "default_true")]
    pub restrict_to_workspace: bool,
    #[serde(default = "default_exec_max_capture_bytes")]
    pub exec_max_capture_bytes: usize,
    #[serde(default = "default_max_session_messages")]
    pub max_session_messages: usize,
    // `context_collapse_after_turns` is the pre-#1017 name; kept as a serde
    // alias so existing config files continue to deserialize.
    #[serde(
        default = "default_context_collapse_after_tool_calls",
        alias = "context_collapse_after_turns"
    )]
    pub context_collapse_after_tool_calls: u32,
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
    /// How many most-recent turns the spilling ceiling tail-pins (#1045).
    #[serde(default = "default_pin_recent_turns")]
    pub pin_recent_turns: u32,
    /// Count-based conversation-message collapse threshold (#1046).
    /// `u32::MAX` disables (conservative default until tuned by observation).
    #[serde(default = "default_context_collapse_after_messages")]
    pub context_collapse_after_messages: u32,
    /// Effort level for 4.6 models (`low`/`medium`/`high`/`max`).
    /// Defaults to `None`; provider applies `low` for 4.6 models when unset.
    #[serde(default)]
    pub effort: Option<String>,
    /// Optional command allowlist. When set, only commands whose first token
    /// is in this list are permitted by the sandbox. When `None`, the sandbox
    /// falls back to the dangerous-command denylist only.
    #[serde(default)]
    pub command_allowlist: Option<Vec<String>>,
}

impl Default for AgentDefaults {
    fn default() -> Self {
        Self {
            workspace: default_workspace(),
            model: default_model(),
            max_tokens: default_max_tokens(),
            temperature: default_temperature(),
            max_tool_iterations: default_max_tool_iterations(),
            restrict_to_workspace: true,
            exec_max_capture_bytes: default_exec_max_capture_bytes(),
            max_session_messages: default_max_session_messages(),
            context_collapse_after_tool_calls: default_context_collapse_after_tool_calls(),
            max_context_tokens: default_max_context_tokens(),
            pin_recent_turns: default_pin_recent_turns(),
            context_collapse_after_messages: default_context_collapse_after_messages(),
            effort: None,
            command_allowlist: None,
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
}

impl std::fmt::Debug for ProviderEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderEntry")
            .field("api_key", &"[REDACTED]")
            .field("api_base", &self.api_base)
            .field("disable_codex_routing", &self.disable_codex_routing)
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
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolsConfig {
    #[serde(default)]
    pub web: WebToolConfig,
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
    "gpt-5.5".to_string()
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
fn default_context_collapse_after_tool_calls() -> u32 {
    // Once the session accumulates more than this many tool calls, the oldest
    // tool results get collapsed to a `recall(spill_id)` stub and their full
    // content spilled to disk. Keeps the hot context small on long sessions;
    // the agent can retrieve spilled content via the `recall` tool when needed.
    50
}
fn default_pin_recent_turns() -> u32 {
    2
}
fn default_context_collapse_after_messages() -> u32 {
    // Keep the 50 most recent conversation (assistant+user) messages in full;
    // older ones collapse to recall() stubs — mirrors the tool-call default.
    50
}
fn default_max_context_tokens() -> usize {
    // Application-level pruning ceiling. Sized well below GPT-5.5's
    // ~1M token window on purpose: a smaller hot-context target
    // keeps latency and cost predictable on long sessions, with
    // older tool output already spilled (see
    // `default_context_collapse_after_tool_calls`) and the hard-drop
    // window dropping oldest non-pinned messages once we breach it.
    200_000
}
fn default_true() -> bool {
    true
}
fn default_max_results() -> u32 {
    5
}
impl Config {
    /// Load config from a JSON file at the given path.
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
        let config: Config = serde_json::from_str(&content).map_err(ConfigError::Parse)?;
        config.validate_effort()?;
        Ok(config)
    }

    /// Reject an unrecognised `agents.defaults.effort` at configuration time
    /// with an error naming every valid value (#1066).
    fn validate_effort(&self) -> Result<(), ConfigError> {
        if let Some(effort) = self.agents.defaults.effort.as_deref()
            && crate::domain::provider::EffortLevel::parse(effort).is_none()
        {
            return Err(ConfigError::InvalidEffort(effort.to_string()));
        }
        Ok(())
    }

    /// Load config from a JSON file, then apply environment variable overrides.
    ///
    /// Environment variable naming convention: `QUECTO_AGENTS_DEFAULTS_MODEL`
    /// maps to `config.agents.defaults.model`.
    pub fn load_with_env(
        path: &str,
        env_overrides: &HashMap<String, String>,
    ) -> Result<Self, ConfigError> {
        let mut config = Self::load(path)?;
        Self::apply_env_overrides(&mut config, env_overrides);
        config.validate_effort()?;
        Ok(config)
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
    /// - `QUECTO_AGENTS_DEFAULTS_EFFORT` → agents.defaults.effort
    /// - `OPENAI_API_KEY` → providers.openai.api_key
    /// - `ANTHROPIC_API_KEY` → providers.anthropic.api_key
    fn apply_env_overrides(config: &mut Config, env: &HashMap<String, String>) {
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
        crate::infrastructure::tools::path_utils::expand_tilde(&self.agents.defaults.workspace)
            .to_string_lossy()
            .to_string()
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(String, std::io::Error),
    Parse(serde_json::Error),
    /// Unrecognised `agents.defaults.effort` value (#1066).
    InvalidEffort(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(path, err) => {
                write!(f, "failed to read config file '{}': {}", path, err)
            }
            ConfigError::Parse(err) => write!(f, "failed to parse config: {}", err),
            ConfigError::InvalidEffort(v) => write!(
                f,
                "invalid effort level '{}'; expected one of: {}",
                v,
                crate::domain::provider::EffortLevel::VALID_VALUES
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
#[path = "config_effort_1066_tests.rs"]
mod effort_1066_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_deserialize_full_config() {
        let json = r#"{
            "agents": {
                "defaults": {
                    "model": "gpt-4",
                    "max_tokens": 4096
                }
            },
            "providers": {
                "openai": {
                    "api_key": "sk-test-123"
                }
            }
        }"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.agents.defaults.model, "gpt-4");
        assert_eq!(config.agents.defaults.max_tokens, 4096);
        assert_eq!(config.providers.openai.api_key, "sk-test-123");
    }

    #[test]
    fn test_deserialize_empty_uses_defaults() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(config.agents.defaults.model, "gpt-5.5");
        assert_eq!(config.agents.defaults.max_tokens, 8192);
        assert!((config.agents.defaults.temperature - 0.7).abs() < f32::EPSILON);
        assert_eq!(config.agents.defaults.workspace, "~/.quecto/workspace");
        assert_eq!(config.agents.defaults.max_tool_iterations, 999_999);
        assert!(config.agents.defaults.restrict_to_workspace);
    }

    #[test]
    fn test_agent_defaults_has_no_command_allowlist() {
        let defaults = AgentDefaults::default();
        assert_eq!(defaults.command_allowlist, None);
    }

    #[test]
    fn test_command_allowlist_deserializes_from_config() {
        let json = r#"{
            "agents": {
                "defaults": {
                    "command_allowlist": ["echo", "ls", "cat"]
                }
            }
        }"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(
            config.agents.defaults.command_allowlist,
            Some(vec![
                "echo".to_string(),
                "ls".to_string(),
                "cat".to_string()
            ])
        );
    }

    #[test]
    fn test_deserialize_legacy_exec_fields_ignored() {
        // Old configs may still carry the removed nsjail/network keys; serde
        // ignores unknown fields, so they deserialize without error.
        let json = r#"{
            "tools": {
                "exec": {
                    "isolation": "nsjail",
                    "nsjail_binary": "/usr/bin/nsjail",
                    "network_passthrough": true
                }
            }
        }"#;
        let config: Config = serde_json::from_str(json).unwrap();
        // Sandbox confinement is independent and still defaults on.
        assert!(config.agents.defaults.restrict_to_workspace);
    }

    #[test]
    fn test_load_from_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(
            tmp,
            r#"{{ "agents": {{ "defaults": {{ "model": "claude-opus-4-5" }} }} }}"#
        )
        .unwrap();
        let config = Config::load(tmp.path().to_str().unwrap()).unwrap();
        assert_eq!(config.agents.defaults.model, "claude-opus-4-5");
        // defaults still applied for missing fields
        assert_eq!(config.agents.defaults.max_tokens, 8192);
    }

    #[test]
    fn test_load_missing_file_returns_default() {
        // Zero-config: a missing config file yields the default config rather
        // than an error (no onboarding step required).
        let config = Config::load("/nonexistent/path/config.json").unwrap();
        assert_eq!(
            config.agents.defaults.model,
            Config::default().agents.defaults.model
        );
    }

    #[test]
    fn test_load_missing_file_with_env_applies_overrides_on_default() {
        // Env overrides apply on top of defaults even with no config file.
        let mut env = HashMap::new();
        env.insert(
            "QUECTO_AGENTS_DEFAULTS_MODEL".to_string(),
            "env/model".to_string(),
        );
        let config = Config::load_with_env("/nonexistent/path/config.json", &env).unwrap();
        assert_eq!(config.agents.defaults.model, "env/model");
    }

    #[test]
    fn test_load_invalid_json() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "not valid json {{").unwrap();
        let result = Config::load(tmp.path().to_str().unwrap());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("failed to parse config"));
    }

    #[test]
    fn test_env_override_model() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(
            tmp,
            r#"{{ "agents": {{ "defaults": {{ "model": "gpt-4" }} }} }}"#
        )
        .unwrap();

        let mut env = HashMap::new();
        env.insert(
            "QUECTO_AGENTS_DEFAULTS_MODEL".to_string(),
            "claude-opus-4-5".to_string(),
        );

        let config = Config::load_with_env(tmp.path().to_str().unwrap(), &env).unwrap();
        assert_eq!(config.agents.defaults.model, "claude-opus-4-5");
    }

    #[test]
    fn test_env_override_max_tokens() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "{{}}").unwrap();

        let mut env = HashMap::new();
        env.insert(
            "QUECTO_AGENTS_DEFAULTS_MAX_TOKENS".to_string(),
            "2048".to_string(),
        );

        let config = Config::load_with_env(tmp.path().to_str().unwrap(), &env).unwrap();
        assert_eq!(config.agents.defaults.max_tokens, 2048);
    }

    #[test]
    fn test_env_override_provider_key() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "{{}}").unwrap();

        let mut env = HashMap::new();
        env.insert("OPENAI_API_KEY".to_string(), "sk-from-env".to_string());

        let config = Config::load_with_env(tmp.path().to_str().unwrap(), &env).unwrap();
        assert_eq!(config.providers.openai.api_key, "sk-from-env");
    }

    #[test]
    fn test_workspace_path_tilde_expansion() {
        let config: Config = serde_json::from_str("{}").unwrap();
        let ws = config.workspace_path();
        assert!(ws.starts_with('/'), "should start with /: {ws}");
        assert!(
            ws.ends_with(".quecto/workspace"),
            "should end with .quecto/workspace: {ws}"
        );
    }

    #[test]
    fn test_workspace_path_absolute_no_expansion() {
        let json = r#"{
            "agents": {
                "defaults": {
                    "workspace": "/opt/quecto/workspace"
                }
            }
        }"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.workspace_path(), "/opt/quecto/workspace");
    }

    #[test]
    fn test_env_override_invalid_number_ignored() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "{{}}").unwrap();

        let mut env = HashMap::new();
        env.insert(
            "QUECTO_AGENTS_DEFAULTS_MAX_TOKENS".to_string(),
            "not_a_number".to_string(),
        );

        let config = Config::load_with_env(tmp.path().to_str().unwrap(), &env).unwrap();
        // Should keep the default since parse failed
        assert_eq!(config.agents.defaults.max_tokens, 8192);
    }

    #[test]
    fn test_provider_entry_debug_redacts_api_key() {
        let entry = ProviderEntry {
            api_key: "sk-secret-key-12345".to_string(),
            api_base: "https://api.openai.com/v1".to_string(),
            disable_codex_routing: false,
        };
        let debug = format!("{:?}", entry);
        assert!(!debug.contains("sk-secret-key-12345"));
    }

    #[test]
    fn test_default_max_session_messages() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(config.agents.defaults.max_session_messages, 200);
    }

    #[test]
    fn test_default_max_context_tokens() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(config.agents.defaults.max_context_tokens, 200_000);
    }

    #[test]
    fn test_default_context_collapse_after_tool_calls_is_50() {
        // #1017: collapse triggers after a configurable number of tool calls,
        // default 50 — pin the default in code, not only in docs.
        assert_eq!(default_context_collapse_after_tool_calls(), 50);
        let config: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(config.agents.defaults.context_collapse_after_tool_calls, 50);
        assert_eq!(
            AgentDefaults::default().context_collapse_after_tool_calls,
            50
        );
    }

    #[test]
    fn test_context_collapse_legacy_turns_alias_deserializes() {
        // Pre-#1017 config files used `context_collapse_after_turns`; the serde
        // alias keeps them working.
        let json = r#"{
            "agents": { "defaults": { "context_collapse_after_turns": 12 } }
        }"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.agents.defaults.context_collapse_after_tool_calls, 12);
    }

    #[test]
    fn test_deserialize_max_session_messages_override() {
        let json = r#"{
            "agents": { "defaults": { "max_session_messages": 12 } }
        }"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.agents.defaults.max_session_messages, 12);
    }

    #[test]
    fn test_openai_compatible_endpoints_deserialize() {
        let json = r#"{
            "providers": {
                "openai_compatible": {
                    "endpoints": [
                        {
                            "prefix": "spark",
                            "api_base": "http://127.0.0.1:8000/v1",
                            "api_key": "sk-spark",
                            "allow_remote_http": true
                        }
                    ]
                }
            }
        }"#;
        let config: Config = serde_json::from_str(json).unwrap();
        let endpoint = &config.providers.openai_compatible.endpoints[0];
        assert_eq!(endpoint.prefix, "spark");
        assert_eq!(endpoint.api_base, "http://127.0.0.1:8000/v1");
        assert_eq!(endpoint.api_key, "sk-spark");
        assert!(endpoint.allow_remote_http);
    }

    #[test]
    fn test_openai_disable_codex_routing_deserializes() {
        let json = r#"{
            "providers": {
                "openai": {
                    "api_key": "sk-custom",
                    "api_base": "http://127.0.0.1:8000/v1",
                    "disable_codex_routing": true
                }
            }
        }"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert!(config.providers.openai.disable_codex_routing);
    }

    #[test]
    fn test_legacy_config_with_removed_sections_still_deserializes() {
        // Guard against regressions: existing config.json files may contain
        // telegram, heartbeat, gateway, health, voice, and cron sections that
        // were removed in #317. serde's default handling must silently ignore
        // these unknown fields.
        let json = r#"{
            "agents": { "defaults": { "model": "gpt-4" } },
            "channels": { "telegram": { "enabled": true, "token": "123:ABC" } },
            "heartbeat": { "enabled": true, "interval": 300 },
            "gateway": { "host": "0.0.0.0", "port": 8080 },
            "health": { "enabled": true, "port": 9090 },
            "voice": { "groq": { "api_key": "gsk-test" } }
        }"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.agents.defaults.model, "gpt-4");
    }
}
