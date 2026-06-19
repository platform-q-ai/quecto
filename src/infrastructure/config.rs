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
    #[serde(default = "default_context_collapse_after_turns")]
    pub context_collapse_after_turns: u32,
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
    /// Effort level for 4.6 models (`low`/`medium`/`high`/`max`).
    /// Defaults to `None`; provider applies `low` for 4.6 models when unset.
    #[serde(default)]
    pub effort: Option<String>,
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
            context_collapse_after_turns: default_context_collapse_after_turns(),
            max_context_tokens: default_max_context_tokens(),
            effort: None,
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
    #[serde(default)]
    pub auth_method: String,
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
    pub auth_method: String,
    #[serde(default)]
    pub allow_remote_http: bool,
}

impl std::fmt::Debug for ProviderEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderEntry")
            .field("api_key", &"[REDACTED]")
            .field("api_base", &self.api_base)
            .field("auth_method", &self.auth_method)
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
            .field("auth_method", &self.auth_method)
            .field("allow_remote_http", &self.allow_remote_http)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolsConfig {
    #[serde(default)]
    pub web: WebToolConfig,
    #[serde(default)]
    pub exec: ExecToolConfig,
}

/// Configuration for the `bash`/exec tool.
///
/// Commands run natively in the workspace; process/network isolation is
/// delegated to the deployment (e.g. running Quecto in a container). The
/// in-process sandbox (see `agents.defaults.restrict_to_workspace` and the
/// `--no-sandbox` flag) still confines file and command access.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExecToolConfig {}

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
fn default_context_collapse_after_turns() -> u32 {
    // Tool results older than this many turns get collapsed to a
    // `recall(spill_id)` stub and their full content spilled to disk.
    // Keeps the hot context small on long sessions; the agent can
    // retrieve spilled content via the `recall` tool when needed.
    50
}
fn default_max_context_tokens() -> usize {
    // Application-level pruning ceiling. Sized well below GPT-5.5's
    // ~1M token window on purpose: a smaller hot-context target
    // keeps latency and cost predictable on long sessions, with
    // older tool output already spilled (see
    // `default_context_collapse_after_turns`) and the hard-drop
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
    pub fn load(path: &str) -> Result<Self, ConfigError> {
        let content =
            std::fs::read_to_string(path).map_err(|e| ConfigError::Io(path.to_string(), e))?;
        let config: Config = serde_json::from_str(&content).map_err(ConfigError::Parse)?;
        Ok(config)
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
    /// - `QUECTO_PROVIDERS_OPENAI_API_KEY` → providers.openai.api_key
    /// - `QUECTO_PROVIDERS_ANTHROPIC_API_KEY` → providers.anthropic.api_key
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
        if let Some(v) = env.get("QUECTO_PROVIDERS_OPENAI_API_KEY") {
            config.providers.openai.api_key = v.clone();
        }
        if let Some(v) = env.get("QUECTO_PROVIDERS_ANTHROPIC_API_KEY") {
            config.providers.anthropic.api_key = v.clone();
        }
        if let Some(v) = env.get("QUECTO_AGENTS_DEFAULTS_EFFORT") {
            if crate::domain::provider::EffortLevel::parse(v).is_some() {
                config.agents.defaults.effort = Some(v.clone());
            }
            // Invalid values are silently ignored (same as invalid MAX_TOKENS).
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
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(path, err) => {
                write!(f, "failed to read config file '{}': {}", path, err)
            }
            ConfigError::Parse(err) => write!(f, "failed to parse config: {}", err),
        }
    }
}

impl std::error::Error for ConfigError {}

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
    fn test_load_missing_file() {
        let result = Config::load("/nonexistent/path/config.json");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("failed to read config file"));
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
        env.insert(
            "QUECTO_PROVIDERS_OPENAI_API_KEY".to_string(),
            "sk-from-env".to_string(),
        );

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
            auth_method: "token".to_string(),
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
