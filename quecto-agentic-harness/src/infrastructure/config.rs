use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

const MAX_WORKFLOW_STEP_FILE_BYTES: u64 = 64 * 1024;

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
    /// Named container configs (#1410): each entry is a complete,
    /// self-contained definition of a container working context (repository,
    /// auth, and runtime mechanics all live in the config's own argv).
    /// Exactly one entry must carry `"default": true` when any are defined —
    /// validated at load time, not at spawn time.
    #[serde(default)]
    pub container_configs: HashMap<String, ContainerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ContainerConfig {
    /// Marks the config `container: true` selects. The label travels with
    /// the entry when copied between config files.
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub create: Vec<String>,
    #[serde(default)]
    pub cleanup: Vec<String>,
    /// Argv for joining an existing environment (#1369 slice 2).
    #[serde(default)]
    pub exec: Vec<String>,
    /// Argv for stopping an environment (#1369 slice 2).
    #[serde(default)]
    pub kill: Vec<String>,
    /// Argv for the post-mortem inspect of a dead member (#1369 slice 3).
    #[serde(default)]
    pub inspect: Vec<String>,
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
    /// Deprecated compatibility key. Filesystem workspace restriction was removed;
    /// old configs may still contain this key, but it is ignored.
    #[serde(default, rename = "restrict_to_workspace", skip_serializing)]
    pub _deprecated_restrict_to_workspace: Option<bool>,
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
            context_collapse_after_tool_calls: default_context_collapse_after_tool_calls(),
            max_context_tokens: default_max_context_tokens(),
            pin_recent_turns: default_pin_recent_turns(),
            context_collapse_after_messages: default_context_collapse_after_messages(),
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
    #[serde(default)]
    pub python_lab: crate::infrastructure::tools::python_lab::PythonLabToolConfig,
    /// Durable catalogue-backed user policy preferences, keyed by stable tool id.
    #[serde(default)]
    pub policy: ToolPolicyConfig,
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
    pub scope: crate::domain::tool_descriptor::ProfileAvailabilityScope,
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
        let mut value: serde_json::Value =
            serde_json::from_str(&content).map_err(ConfigError::Parse)?;
        // Honest breaking-window signal, not a compat shim: the pre-#1410 key
        // would otherwise be silently ignored and containers would quietly
        // become "none configured".
        if value.get("container_scripts").is_some() {
            return Err(ConfigError::ContainerConfigs(
                "the `container_scripts` key was renamed to `container_configs` (#1410); \
                 entries are now a flat map of container configs with exactly one labeled \
                 \"default\": true — see docs/container-runtimes.md"
                    .into(),
            ));
        }
        let resolved_references = resolve_workflow_step_entries(&mut value, Path::new(path))?;
        // Deserializing the resolved Value loses line/column error context, so
        // only pay that cost when a reference was actually substituted.
        let config: Config = if resolved_references {
            serde_json::from_value(value).map_err(ConfigError::Parse)?
        } else {
            serde_json::from_str(&content).map_err(ConfigError::Parse)?
        };
        config.validate_effort()?;
        config.validate_container_configs()?;
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

    /// Exactly one container config must be labeled `"default": true` when
    /// any are defined (#1410). Zero or multiple defaults fail here, at load
    /// time, naming the configured entries so the caller can fix the file.
    fn validate_container_configs(&self) -> Result<(), ConfigError> {
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
        let mut config = Self::load(path)?;
        Self::apply_env_overrides(&mut config, env_overrides);
        config.validate_effort()?;
        config.validate_container_configs()?;
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

// Workflow template directory discovery (slice 2) lives beside this module;
// it reuses the private step-reference resolver below.
#[path = "config_discovery.rs"]
mod discovery;
pub use discovery::{
    WorkflowTemplateDiscovery, discover_workflow_templates, load_workflow_templates_from_dir,
};

const WORKFLOW_STEP_FIELDS: &[&str] = &["key", "label", "phase", "guidance"];

/// Returns `true` when resolution replaced at least one reference entry;
/// callers use that to keep the cheaper (and line/column-preserving) text
/// deserialization path for configs without references.
fn resolve_workflow_step_entries(
    config: &mut serde_json::Value,
    config_path: &Path,
) -> Result<bool, ConfigError> {
    let Some(templates) = config
        .get_mut("workflow")
        .and_then(|workflow| workflow.get_mut("templates"))
        .and_then(serde_json::Value::as_array_mut)
    else {
        return Ok(false);
    };
    // Bound file loads before any reference is resolved; the engine re-checks
    // both limits post-load with full template validation.
    if templates.len() > crate::domain::workflow::MAX_TEMPLATE_COUNT {
        return Err(ConfigError::WorkflowStep(format!(
            "too many workflow templates: {} > {}",
            templates.len(),
            crate::domain::workflow::MAX_TEMPLATE_COUNT
        )));
    }
    // `Path::new("config.json").parent()` is `Some("")`, not `None`.
    let base_dir = match config_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    let mut resolved = false;
    for template in templates {
        let id = template
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let Some(steps) = template
            .get_mut("steps")
            .and_then(serde_json::Value::as_array_mut)
        else {
            continue;
        };
        if steps.len() > crate::domain::workflow::MAX_STEPS_PER_TEMPLATE {
            return Err(ConfigError::WorkflowStep(format!(
                "template '{id}' has too many steps: {} > {}",
                steps.len(),
                crate::domain::workflow::MAX_STEPS_PER_TEMPLATE
            )));
        }
        for entry in steps {
            resolved |= is_workflow_step_reference(entry);
            *entry = resolve_workflow_step_entry(entry.take(), base_dir)?;
        }
    }
    Ok(resolved)
}

/// A reference object is one carrying `ref` whose other keys are all step
/// fields — including when every field is overridden, so the referenced file
/// is still loaded and validated. The one carve-out: an object that is
/// already a complete inline step (`key` + `label` + `phase`) AND carries
/// keys outside the step fields stays inline, so pre-existing configs using
/// `ref` as free-form step metadata keep working.
fn is_workflow_step_reference_object(object: &serde_json::Map<String, serde_json::Value>) -> bool {
    object.contains_key("ref")
        && (object
            .keys()
            .all(|key| key == "ref" || WORKFLOW_STEP_FIELDS.contains(&key.as_str()))
            || !(object.contains_key("key")
                && object.contains_key("label")
                && object.contains_key("phase")))
}

fn is_workflow_step_reference(entry: &serde_json::Value) -> bool {
    match entry {
        serde_json::Value::String(_) => true,
        serde_json::Value::Object(object) => is_workflow_step_reference_object(object),
        _ => false,
    }
}

fn resolve_workflow_step_entry(
    entry: serde_json::Value,
    base_dir: &Path,
) -> Result<serde_json::Value, ConfigError> {
    match entry {
        serde_json::Value::String(reference) => load_workflow_step(base_dir, &reference),
        serde_json::Value::Object(mut object) if is_workflow_step_reference_object(&object) => {
            let reference = object
                .remove("ref")
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or_else(|| ConfigError::WorkflowStep("`ref` must be a string".into()))?;
            reject_unknown_keys(&object, WORKFLOW_STEP_FIELDS, "step reference")?;
            let mut step = load_workflow_step(base_dir, &reference)?;
            let target = step.as_object_mut().expect("loaded step is an object");
            target.extend(object);
            serde_json::from_value::<crate::domain::workflow::WorkflowTemplateStep>(step.clone())
                .map_err(|error| ConfigError::WorkflowStep(format!("{reference}: {error}")))?;
            Ok(step)
        }
        serde_json::Value::Object(object) => Ok(serde_json::Value::Object(object)),
        _ => Err(ConfigError::WorkflowStep(
            "step entry must be a string reference, reference object, or step object".into(),
        )),
    }
}

fn load_workflow_step(base_dir: &Path, reference: &str) -> Result<serde_json::Value, ConfigError> {
    let mut relative = PathBuf::from(reference);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ConfigError::WorkflowStep(format!(
            "step reference must remain within the config directory: {reference}"
        )));
    }
    if relative.extension().is_none() {
        relative.set_extension("json");
    }
    let path = base_dir.join(relative);
    let canonical_base = base_dir
        .canonicalize()
        .map_err(|error| ConfigError::WorkflowStep(format!("{}: {error}", base_dir.display())))?;
    let canonical_path = path
        .canonicalize()
        .map_err(|error| ConfigError::WorkflowStep(format!("{}: {error}", path.display())))?;
    if !canonical_path.starts_with(&canonical_base) {
        return Err(ConfigError::WorkflowStep(format!(
            "{}: step reference escapes config directory",
            path.display()
        )));
    }
    let size = canonical_path
        .metadata()
        .map_err(|error| ConfigError::WorkflowStep(format!("{}: {error}", path.display())))?
        .len();
    if size > MAX_WORKFLOW_STEP_FILE_BYTES {
        return Err(ConfigError::WorkflowStep(format!(
            "{}: step file is too large: {size} > {MAX_WORKFLOW_STEP_FILE_BYTES}",
            path.display()
        )));
    }
    let content = std::fs::read_to_string(&canonical_path)
        .map_err(|error| ConfigError::WorkflowStep(format!("{}: {error}", path.display())))?;
    let value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|error| ConfigError::WorkflowStep(format!("{}: {error}", path.display())))?;
    let object = value.as_object().ok_or_else(|| {
        ConfigError::WorkflowStep(format!("{}: expected a step object", path.display()))
    })?;
    if object.contains_key("ref") {
        return Err(ConfigError::WorkflowStep(format!(
            "{}: expected a step object; recursive references are not allowed",
            path.display()
        )));
    }
    reject_unknown_keys(object, WORKFLOW_STEP_FIELDS, &path.display().to_string())?;
    serde_json::from_value::<crate::domain::workflow::WorkflowTemplateStep>(value.clone())
        .map_err(|error| ConfigError::WorkflowStep(format!("{}: {error}", path.display())))?;
    Ok(value)
}

fn reject_unknown_keys(
    object: &serde_json::Map<String, serde_json::Value>,
    allowed: &[&str],
    context: &str,
) -> Result<(), ConfigError> {
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(ConfigError::WorkflowStep(format!(
            "{context}: unknown field `{key}`"
        )));
    }
    Ok(())
}

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
        }
    }
}

impl std::error::Error for ConfigError {}

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
#[path = "config_tests.rs"]
mod tests;
