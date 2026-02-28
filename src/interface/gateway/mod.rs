// Gateway service: starts channels, heartbeat, cron, and health server.

mod services;
mod telegram;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

use std::path::PathBuf;
use std::sync::Arc;

use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::agent::AgentLoop;
use crate::domain::channel::Channel;
use crate::domain::provider::LlmProvider;
use crate::domain::session::SessionStore;
use crate::infrastructure::auth::credential_store::{Credential, CredentialStore};
use crate::infrastructure::bus::MessageBus;
use crate::infrastructure::channels::telegram::TelegramChannel;
use crate::infrastructure::config::Config;
use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
use crate::infrastructure::persistence::cron_store::FileCronStore;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::providers;
use crate::infrastructure::providers::fallback::FallbackProvider;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::cron_tool::CronTool;
use crate::infrastructure::tools::message::MessageTool;
use crate::infrastructure::tools::recall::RecallTool;
use crate::infrastructure::tools::registry::ToolRegistryImpl;
use crate::infrastructure::tools::spawn::SpawnTool;
use crate::infrastructure::tools::web_search::WebSearchTool;
use crate::interface::shared::{
    CodingCoordinatorScopePolicy, build_coding_tool, build_gateway_system_prompt,
    gateway_background_coding_coordinator_scope,
};

use tokio::sync::mpsc;

use crate::infrastructure::bus::{InboundMessage, OutboundMessage};
use services::InboundProcessorContext;

/// Gateway error type.
#[derive(Debug)]
pub enum GatewayError {
    Config(String),
    NoProviders,
    Runtime(String),
}

impl std::fmt::Display for GatewayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GatewayError::Config(msg) => write!(f, "config error: {}", msg),
            GatewayError::NoProviders => write!(f, "no LLM providers configured"),
            GatewayError::Runtime(msg) => write!(f, "runtime error: {}", msg),
        }
    }
}

impl std::error::Error for GatewayError {}

/// Bundles the runtime components for the concurrent event loop.
pub(super) struct EventLoopContext {
    pub(super) inbound_tx: mpsc::Sender<InboundMessage>,
    pub(super) inbound_rx: mpsc::Receiver<InboundMessage>,
    pub(super) outbound_tx: mpsc::Sender<OutboundMessage>,
    pub(super) outbound_rx: mpsc::Receiver<OutboundMessage>,
    pub(super) agent: Arc<dyn AgentLoop>,
    pub(super) session_store: Arc<dyn SessionStore>,
    pub(super) outbound_channel: Arc<dyn Channel>,
    pub(super) telegram_poller: Arc<TelegramChannel>,
    pub(super) config: Config,
    pub(super) base_dir: PathBuf,
    pub(super) workspace: PathBuf,
    pub(super) cron_store: Arc<FileCronStore>,
    pub(super) provider_for_inbound: Arc<dyn LlmProvider>,
    /// Whether to allow insecure (HTTP) voice API base URLs.
    /// Read once at startup from env var, threaded through to avoid mid-run env reads.
    pub(super) allow_insecure_voice: bool,
}

impl EventLoopContext {
    /// Run the concurrent event loop until shutdown.
    ///
    /// Spawns background services (health, heartbeat, cron) as tasks and
    /// selects on the core messaging pipeline plus shutdown signal. When
    /// any core task completes or ctrl-c fires, spawned tasks are aborted.
    async fn run(self) {
        // Background services — store handles so we can abort on shutdown.
        let h_health = tokio::spawn(Gateway::run_health_server(self.config.health.clone()));
        let h_heartbeat = tokio::spawn(Gateway::run_heartbeat(
            self.config.heartbeat.clone(),
            self.agent.clone(),
            self.workspace,
        ));
        let h_cron = tokio::spawn(Gateway::run_cron_tick(
            self.cron_store.clone(),
            self.agent.clone(),
            self.config.tools.cron.exec_timeout_minutes,
        ));
        let max_session_messages = self
            .config
            .agents
            .defaults
            .max_session_messages
            .clamp(10, 1000);
        let polling_config = self.config.clone();

        // Core messaging pipeline — select until one stops or shutdown.
        tokio::select! {
            _ = Gateway::run_telegram_polling(
                self.telegram_poller, self.inbound_tx, polling_config, self.allow_insecure_voice,
            ) => {
                tracing::info!("Telegram polling stopped");
            }
            _ = Gateway::run_inbound_processor(
                self.inbound_rx,
                InboundProcessorContext {
                    agent: self.agent,
                    agent_builder: Some(Arc::new(services::InboundAgentBuilder {
                        config: self.config.clone(),
                        base_dir: self.base_dir.clone(),
                        provider: self.provider_for_inbound.clone(),
                        cron_store: self.cron_store.clone(),
                    })),
                    session_store: self.session_store,
                    outbound_tx: self.outbound_tx,
                    max_session_messages,
                    system_prompt: build_gateway_system_prompt(
                        &self.base_dir,
                        self.config.tools.coding.coordinator_mode,
                    ),
                },
            ) => {
                tracing::info!("Inbound processor stopped");
            }
            _ = Gateway::run_outbound_dispatcher(self.outbound_rx, self.outbound_channel) => {
                tracing::info!("Outbound dispatcher stopped");
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutdown signal received");
            }
        }

        // Cancel background services on shutdown.
        h_health.abort();
        h_heartbeat.abort();
        h_cron.abort();
    }
}

/// The gateway orchestrator: wires all components together and runs async tasks.
pub struct Gateway {
    config: Config,
    base_dir: PathBuf,
}

impl Gateway {
    /// Create a new gateway from a loaded config and base directory.
    pub fn new(config: Config, base_dir: PathBuf) -> Self {
        Self { config, base_dir }
    }

    /// Build all components and run the gateway until shutdown.
    pub async fn run(&self) -> Result<(), GatewayError> {
        // Initialize tracing subscriber so tracing::info!/warn!/error! produce output.
        // Defaults to INFO level; override with RUST_LOG env var (e.g. RUST_LOG=debug).
        // Uses try_init() to avoid panicking if a subscriber is already set (e.g. in tests
        // or if Gateway::run() is retried after a transient failure).
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .try_init();
        let workspace = self.resolve_workspace();

        // Load credential snapshot once for both provider resolution and readiness check.
        let store = CredentialStore::new(&self.base_dir);
        let creds = store.load_snapshot().unwrap_or_default();
        let needs_reauth = check_provider_readiness(&creds);
        for provider_name in &needs_reauth {
            tracing::warn!(
                provider = provider_name.as_str(),
                "credential expired — run `quecto auth login` to re-authenticate"
            );
        }

        let provider_impl = Arc::new(self.build_fallback_provider(&creds)?);
        let provider: Arc<dyn LlmProvider> = provider_impl.clone();
        let cron_store = Arc::new(FileCronStore::new(&self.base_dir));
        let (agent_impl, mut bus) =
            self.build_agent(workspace.clone(), provider.clone(), cron_store.clone());
        let agent: Arc<dyn AgentLoop> = Arc::new(agent_impl);

        let info = agent.info();
        let model = &self.config.agents.defaults.model;
        tracing::info!(tools = info.tool_count, model, "quecto gateway starting");

        // Read insecure voice flag once at startup (not per-request).
        let allow_insecure_voice = matches!(
            std::env::var(telegram::ALLOW_INSECURE_VOICE_API_BASE_ENV)
                .ok()
                .as_deref(),
            Some("1") | Some("true") | Some("TRUE") | Some("True")
        );

        let (inbound_tx, inbound_rx, outbound_tx, outbound_rx) = Self::take_channels(&mut bus);
        let telegram = Arc::new(TelegramChannel::new(&self.config.channels.telegram));
        let ctx = EventLoopContext {
            inbound_tx,
            inbound_rx,
            outbound_tx,
            outbound_rx,
            agent,
            session_store: Arc::new(FileSessionStore::new(&self.base_dir)),
            outbound_channel: telegram.clone(),
            telegram_poller: telegram,
            config: self.config.clone(),
            base_dir: self.base_dir.clone(),
            workspace: workspace.clone(),
            cron_store,
            provider_for_inbound: provider,
            allow_insecure_voice,
        };
        ctx.run().await;

        tracing::info!("quecto gateway stopped");
        Ok(())
    }

    /// Extract all channels from the message bus.
    fn take_channels(
        bus: &mut MessageBus,
    ) -> (
        mpsc::Sender<InboundMessage>,
        mpsc::Receiver<InboundMessage>,
        mpsc::Sender<OutboundMessage>,
        mpsc::Receiver<OutboundMessage>,
    ) {
        let outbound_tx = bus.outbound_sender();
        let inbound_tx = bus.inbound_sender();
        let inbound_rx = bus
            .take_inbound_receiver()
            .expect("inbound receiver already taken");
        let outbound_rx = bus
            .take_outbound_receiver()
            .expect("outbound receiver already taken");
        (inbound_tx, inbound_rx, outbound_tx, outbound_rx)
    }

    /// Build the fallback provider from configured API keys.
    ///
    /// Accepts a pre-loaded credential snapshot to maintain the
    /// single-snapshot-at-startup invariant and avoid redundant file reads.
    fn build_fallback_provider(
        &self,
        creds: &std::collections::HashMap<String, Credential>,
    ) -> Result<FallbackProvider, GatewayError> {
        let mut provider_list = Vec::new();
        self.maybe_add_provider(&mut provider_list, "openai", creds)?;
        self.maybe_add_provider(&mut provider_list, "anthropic", creds)?;
        if provider_list.is_empty() {
            return Err(GatewayError::NoProviders);
        }
        Ok(FallbackProvider::new(provider_list))
    }

    /// Try to create a provider and add it to the list.
    ///
    /// Resolves the API key from the credential snapshot (takes priority over config),
    /// falling back to the config file key if no valid credential is stored.
    fn maybe_add_provider(
        &self,
        list: &mut Vec<Arc<dyn LlmProvider>>,
        name: &str,
        creds: &std::collections::HashMap<String, Credential>,
    ) -> Result<(), GatewayError> {
        let (config_key, api_base) = match name {
            "openai" => (
                &self.config.providers.openai.api_key,
                &self.config.providers.openai.api_base,
            ),
            "anthropic" => (
                &self.config.providers.anthropic.api_key,
                &self.config.providers.anthropic.api_base,
            ),
            _ => return Ok(()),
        };
        let api_key = resolve_api_key(config_key, creds, name);
        if api_key.is_empty() {
            return Ok(());
        }
        let base = if api_base.is_empty() {
            None
        } else {
            Some(api_base.clone())
        };
        match providers::create_provider(name, api_key, base) {
            Ok(p) => list.push(p),
            Err(e) => {
                return Err(GatewayError::Config(format!(
                    "{} provider configuration error: {}",
                    name, e
                )));
            }
        }
        Ok(())
    }

    /// Build agent loop with tool registry and message bus.
    fn build_agent(
        &self,
        workspace: PathBuf,
        provider: Arc<dyn LlmProvider>,
        cron_store: Arc<FileCronStore>,
    ) -> (AgentLoopImpl, MessageBus) {
        let sandbox = Sandbox::new(
            Some(workspace.clone()),
            self.config.agents.defaults.restrict_to_workspace,
        );
        let exec_settings = ToolRegistryImpl::exec_registry_settings_from_config(&self.config);
        let mut registry = ToolRegistryImpl::with_core_tools_and_exec_settings(
            workspace.clone(),
            sandbox,
            exec_settings,
        );
        let bus = MessageBus::new(256);
        let outbound_tx = bus.outbound_sender();

        registry.register(Arc::new(MessageTool::new(outbound_tx, None)));
        registry.register(Arc::new(WebSearchTool::new(self.brave_api_key())));
        registry.register(Arc::new(CronTool::new(cron_store)));
        registry.register(Arc::new(SpawnTool::with_base_dir(
            vec![],
            self.config.agents.defaults.restrict_to_workspace,
            self.base_dir.clone(),
        )));
        if gateway_background_coding_coordinator_scope() == CodingCoordinatorScopePolicy::Shared {
            // Driver handle intentionally discarded — the gateway background agent
            // relies on tick-on-access via DriverJobService (run/status calls).
            // A periodic background ticker will be added when the gateway needs
            // to poll running workers independently of tool calls.
            let _ = build_coding_tool(
                &mut registry,
                &workspace,
                &self.base_dir,
                self.config.tools.coding.coordinator_mode,
            );
        }
        let spill_store = Arc::new(FileContextSpillStore::new(self.base_dir.clone()));
        // Shared agent handles cron/heartbeat tasks, not per-user Telegram messages.
        // Per-user messages use InboundAgentBuilder with session-scoped spill stores.
        let session_key = "gateway:cron-heartbeat".to_string();
        registry.register(Arc::new(RecallTool::new(
            spill_store.clone(),
            session_key.clone(),
        )));

        let agent = AgentLoopImpl::new(AgentLoopConfig {
            provider,
            tool_registry: Box::new(registry),
            model: self.config.agents.defaults.model.clone(),
            max_tokens: self.config.agents.defaults.max_tokens,
            temperature: self.config.agents.defaults.temperature,
            spill_store: Some(spill_store),
            session_key,
            context_collapse_after_turns: self.config.agents.defaults.context_collapse_after_turns,
            max_context_tokens: self.config.agents.defaults.max_context_tokens,
        })
        .with_max_tool_iterations(self.config.agents.defaults.max_tool_iterations);

        (agent, bus)
    }

    /// Extract the Brave API key if configured and enabled.
    fn brave_api_key(&self) -> Option<String> {
        let brave = &self.config.tools.web.brave;
        if brave.enabled && !brave.api_key.is_empty() {
            Some(brave.api_key.clone())
        } else {
            None
        }
    }

    fn resolve_workspace(&self) -> PathBuf {
        let ws = self.config.workspace_path();
        PathBuf::from(ws)
    }
}

/// Handle a Telegram bot command (`/start`, `/help`, `/status`).
///
/// Returns `Some(response_text)` if the command is a known bot command,
/// or `None` if the message should be routed to the agent as regular text.
/// Only the first whitespace-delimited token is matched; trailing arguments are ignored
/// (e.g. `/start deep_link_payload` still matches `/start`).
pub fn handle_bot_command(text: &str, config: &Config) -> Option<String> {
    let command = text.split_whitespace().next().unwrap_or("");
    match command {
        "/start" => Some(
            "Welcome to quecto! I'm your personal AI assistant.\n\
             Type a message to chat, or use /help to see available commands."
                .to_string(),
        ),
        "/help" => Some(
            "Available commands:\n\
             /start  — Show welcome message\n\
             /help   — Show this help\n\
             /status — Show bot status\n\n\
             Or just type a message to chat with me."
                .to_string(),
        ),
        "/status" => {
            let model = &config.agents.defaults.model;
            let telegram_status = if config.channels.telegram.enabled {
                "enabled"
            } else {
                "disabled"
            };
            Some(format!(
                "quecto Status\n\
                 Model: {model}\n\
                 Telegram: {telegram_status}"
            ))
        }
        _ => None,
    }
}

// Re-export shared credential resolution functions for backward compatibility.
pub use crate::interface::shared::{check_provider_readiness, resolve_api_key};
