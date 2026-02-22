use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub agents: AgentConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub channels: ChannelsConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub heartbeat: HeartbeatConfig,
    #[serde(default)]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub health: HealthConfig,
    #[serde(default)]
    pub voice: VoiceConfig,
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
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProvidersConfig {
    #[serde(default)]
    pub openai: ProviderEntry,
    #[serde(default)]
    pub anthropic: ProviderEntry,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct ProviderEntry {
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub api_base: String,
    #[serde(default)]
    pub auth_method: String,
}

impl std::fmt::Debug for ProviderEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderEntry")
            .field("api_key", &"[REDACTED]")
            .field("api_base", &self.api_base)
            .field("auth_method", &self.auth_method)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChannelsConfig {
    #[serde(default)]
    pub telegram: TelegramConfig,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct TelegramConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub api_base: String,
    #[serde(default)]
    pub allow_from: Vec<String>,
}

impl std::fmt::Debug for TelegramConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramConfig")
            .field("enabled", &self.enabled)
            .field("token", &"[REDACTED]")
            .field("api_base", &self.api_base)
            .field("allow_from", &self.allow_from)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolsConfig {
    #[serde(default)]
    pub web: WebToolConfig,
    #[serde(default)]
    pub cron: CronToolConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebToolConfig {
    #[serde(default)]
    pub brave: BraveConfig,
    #[serde(default)]
    pub duckduckgo: DuckDuckGoConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronToolConfig {
    #[serde(default = "default_cron_timeout")]
    pub exec_timeout_minutes: u32,
}

impl Default for CronToolConfig {
    fn default() -> Self {
        Self {
            exec_timeout_minutes: default_cron_timeout(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_heartbeat_interval")]
    pub interval: u32,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval: default_heartbeat_interval(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_health_port")]
    pub port: u16,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: default_health_port(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VoiceConfig {
    #[serde(default)]
    pub groq: GroqVoiceConfig,
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct GroqVoiceConfig {
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub api_base: String,
}

impl std::fmt::Debug for GroqVoiceConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroqVoiceConfig")
            .field("api_key", &"[REDACTED]")
            .field("api_base", &self.api_base)
            .finish()
    }
}

fn default_workspace() -> String {
    "~/.quecto/workspace".to_string()
}
fn default_model() -> String {
    "gpt-5.2".to_string()
}
fn default_max_tokens() -> u32 {
    8192
}
fn default_temperature() -> f32 {
    0.7
}
fn default_max_tool_iterations() -> u32 {
    20
}
fn default_exec_max_capture_bytes() -> usize {
    1024 * 1024
}
fn default_true() -> bool {
    true
}
fn default_max_results() -> u32 {
    5
}
fn default_cron_timeout() -> u32 {
    5
}
fn default_heartbeat_interval() -> u32 {
    30
}
fn default_host() -> String {
    "0.0.0.0".to_string()
}
fn default_port() -> u16 {
    8080
}
fn default_health_port() -> u16 {
    9090
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
        if let Some(v) = env.get("QUECTO_PROVIDERS_OPENAI_API_KEY") {
            config.providers.openai.api_key = v.clone();
        }
        if let Some(v) = env.get("QUECTO_PROVIDERS_ANTHROPIC_API_KEY") {
            config.providers.anthropic.api_key = v.clone();
        }
    }

    /// Resolve the workspace path, expanding `~` to the user's home directory.
    pub fn workspace_path(&self) -> String {
        let ws = &self.agents.defaults.workspace;
        if let Some(stripped) = ws.strip_prefix("~/")
            && let Some(home) = dirs::home_dir()
        {
            return home.join(stripped).to_string_lossy().to_string();
        }
        ws.clone()
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
        assert_eq!(config.agents.defaults.model, "gpt-5.2");
        assert_eq!(config.agents.defaults.max_tokens, 8192);
        assert!((config.agents.defaults.temperature - 0.7).abs() < f32::EPSILON);
        assert_eq!(config.agents.defaults.workspace, "~/.quecto/workspace");
        assert_eq!(config.agents.defaults.max_tool_iterations, 20);
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
        };
        let debug = format!("{:?}", entry);
        assert!(!debug.contains("sk-secret-key-12345"));
    }
}
