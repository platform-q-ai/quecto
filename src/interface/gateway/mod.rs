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
use crate::infrastructure::auth::credential_store::CredentialStore;
use crate::infrastructure::bus::MessageBus;
use crate::infrastructure::channels::telegram::TelegramChannel;
use crate::infrastructure::config::Config;
use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
use crate::infrastructure::persistence::cron_store::FileCronStore;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::providers;
use crate::infrastructure::providers::fallback::FallbackProvider;
use crate::infrastructure::providers::refreshable::{RefreshableConfig, RefreshableProvider};
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::cron_tool::CronTool;
use crate::infrastructure::tools::message::MessageTool;
use crate::infrastructure::tools::recall::RecallTool;
use crate::infrastructure::tools::registry::ToolRegistryImpl;
use crate::infrastructure::tools::spawn::SpawnTool;
use crate::infrastructure::tools::web_search::WebSearchTool;

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
    pub(super) spill_store: Arc<dyn crate::domain::session::ContextSpillStore>,
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
    /// Skill prompt loaded once at startup, combined with datetime per-request.
    pub(super) skill_prompt: String,
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
            self.skill_prompt.clone(),
        ));
        let h_cron = tokio::spawn(Gateway::run_cron_tick(services::CronTickContext {
            store: self.cron_store.clone(),
            agent: self.agent.clone(),
            timeout_minutes: self.config.tools.cron.exec_timeout_minutes,
            skill_prompt: self.skill_prompt.clone(),
            outbound_tx: self.outbound_tx.clone(),
            default_send_to: self.config.channels.telegram.default_send_to.clone(),
        }));
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
                self.telegram_poller,
                telegram::TelegramPollingConfig {
                    inbound_tx: self.inbound_tx,
                    config: polling_config,
                    allow_insecure_voice: self.allow_insecure_voice,
                    session_store: self.session_store.clone(),
                    spill_store: self.spill_store.clone(),
                },
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
                    skill_prompt: self.skill_prompt.clone(),
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

        // Load credential store for provider resolution and readiness check.
        // The store is passed to build_fallback_provider which uses async refresh
        // to automatically refresh expired OAuth tokens (issue #254).
        let store = CredentialStore::new(&self.base_dir);
        let creds = store.load_snapshot().unwrap_or_default();
        let needs_reauth = check_provider_readiness(&creds);
        for provider_name in &needs_reauth {
            tracing::warn!(
                provider = provider_name.as_str(),
                "credential expired — attempting OAuth refresh"
            );
        }

        // Validate default_send_to at startup: must be "channel:id" format if set.
        if let Some(ref dst) = self.config.channels.telegram.default_send_to {
            if !dst.contains(':') || dst.ends_with(':') {
                tracing::warn!(
                    default_send_to = dst.as_str(),
                    "channels.telegram.default_send_to does not match expected \
                     'channel:id' format (e.g. 'telegram:123456789'). \
                     Delivery may fail silently at runtime."
                );
            }
        }

        // Shared HTTP client for all providers and HTTP-using tools.
        let http_client = reqwest::Client::new();
        let provider_impl = Arc::new(self.build_fallback_provider(&store, &http_client).await?);
        let provider: Arc<dyn LlmProvider> = provider_impl.clone();
        let cron_store = Arc::new(FileCronStore::new(&self.base_dir));
        let (agent_impl, mut bus) = self.build_agent(
            workspace.clone(),
            provider.clone(),
            (cron_store.clone(), &http_client),
        );
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
        let skill_prompt = crate::interface::shared::load_skill_prompt(&self.base_dir);
        let ctx = EventLoopContext {
            inbound_tx,
            inbound_rx,
            outbound_tx,
            outbound_rx,
            agent,
            session_store: Arc::new(FileSessionStore::new(&self.base_dir)),
            spill_store: Arc::new(FileContextSpillStore::new(self.base_dir.clone())),
            outbound_channel: telegram.clone(),
            telegram_poller: telegram,
            config: self.config.clone(),
            base_dir: self.base_dir.clone(),
            workspace: workspace.clone(),
            cron_store,
            provider_for_inbound: provider,
            allow_insecure_voice,
            skill_prompt,
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
    /// Uses async token refresh so that expired OAuth tokens are automatically
    /// refreshed at startup (issue #254). Accepts a `CredentialStore` reference
    /// to enable refresh-and-persist on expired tokens.
    async fn build_fallback_provider(
        &self,
        store: &CredentialStore,
        http_client: &reqwest::Client,
    ) -> Result<FallbackProvider, GatewayError> {
        let mut provider_list = Vec::new();
        for name in &["openai", "anthropic"] {
            if let Some(p) = self.resolve_provider(name, store, http_client).await? {
                provider_list.push(p);
            }
        }
        if provider_list.is_empty() {
            return Err(GatewayError::NoProviders);
        }
        Ok(FallbackProvider::new(provider_list))
    }

    /// Resolve a single provider by name, returning `None` if no key is configured.
    ///
    /// Uses async token refresh to automatically refresh expired OAuth tokens
    /// (issue #254). OAuth-backed providers are wrapped in [`RefreshableProvider`]
    /// so that 401 errors trigger automatic token refresh mid-session (#255).
    async fn resolve_provider(
        &self,
        name: &str,
        store: &CredentialStore,
        http_client: &reqwest::Client,
    ) -> Result<Option<Arc<dyn LlmProvider>>, GatewayError> {
        let (config_key, api_base) = match name {
            "openai" => (
                &self.config.providers.openai.api_key,
                &self.config.providers.openai.api_base,
            ),
            "anthropic" => (
                &self.config.providers.anthropic.api_key,
                &self.config.providers.anthropic.api_base,
            ),
            _ => return Ok(None),
        };
        let api_key =
            crate::interface::shared::resolve_api_key_with_refresh_async(config_key, store, name)
                .await;
        if api_key.is_empty() {
            return Ok(None);
        }

        let is_oauth = store.get(name).ok().flatten().is_some_and(|c| {
            c.method == crate::infrastructure::auth::credential_store::AuthMethod::OAuth
        });

        let base = if api_base.is_empty() {
            None
        } else {
            Some(api_base.clone())
        };

        let inner = Self::build_single_provider(name, &api_key, &base, http_client)?;

        if is_oauth {
            let store_arc = Arc::new(CredentialStore::new(store.path().parent().unwrap()));
            let factory =
                crate::interface::shared::make_provider_factory(name, base, http_client.clone());
            let refresh_fn = crate::interface::shared::make_oauth_refresh_fn();
            Ok(Some(Arc::new(RefreshableProvider::new(
                RefreshableConfig {
                    inner,
                    store: store_arc,
                    provider_name: name.to_string(),
                    refresh_fn,
                    factory,
                },
            ))))
        } else {
            Ok(Some(inner))
        }
    }

    /// Build a single provider from name, key, base URL, and HTTP client.
    fn build_single_provider(
        name: &str,
        api_key: &str,
        api_base: &Option<String>,
        http_client: &reqwest::Client,
    ) -> Result<Arc<dyn LlmProvider>, GatewayError> {
        if name == "openai" {
            let account_id = crate::infrastructure::auth::oauth::extract_openai_account_id(api_key);
            if let Some(acct) = account_id {
                return Ok(providers::create_codex_provider_with_client(
                    api_key.to_string(),
                    acct,
                    http_client.clone(),
                ));
            }
        }
        let base = api_base.clone();
        providers::create_provider_with_client(name, api_key.to_string(), base, http_client.clone())
            .map_err(|e| {
                GatewayError::Config(format!("{} provider configuration error: {}", name, e))
            })
    }

    /// Build agent loop with tool registry and message bus.
    fn build_agent(
        &self,
        workspace: PathBuf,
        provider: Arc<dyn LlmProvider>,
        ctx: (Arc<FileCronStore>, &reqwest::Client),
    ) -> (AgentLoopImpl, MessageBus) {
        let (cron_store, http_client) = ctx;
        let sandbox = Sandbox::new(
            Some(workspace.clone()),
            self.config.agents.defaults.restrict_to_workspace,
        );
        let exec_settings = ToolRegistryImpl::exec_registry_settings_from_config(&self.config);
        let mut registry =
            ToolRegistryImpl::with_core_tools_and_exec_settings(workspace, sandbox, exec_settings);
        let bus = MessageBus::new(256);
        let outbound_tx = bus.outbound_sender();

        // Wire default_send_to so cron/heartbeat agent tool calls can deliver
        // without an explicit target — closes the gap where deliver_cron_result
        // handles post-processing delivery but in-loop MessageTool calls did not.
        let default_send_to = self.config.channels.telegram.default_send_to.clone();
        registry.register(Arc::new(MessageTool::new(outbound_tx, default_send_to)));
        registry.register(Arc::new(WebSearchTool::with_client(
            self.brave_api_key(),
            http_client.clone(),
        )));
        registry.register(Arc::new(CronTool::new(cron_store)));
        registry.register(Arc::new(SpawnTool::with_base_dir(
            vec![],
            self.config.agents.defaults.restrict_to_workspace,
            self.base_dir.clone(),
        )));
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
            progress_callback: None,
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

/// Sentinel returned by `handle_bot_command` for `/reload` to signal `dispatch_update`
/// that I/O-backed reload logic should run. Defined as a constant to prevent the
/// two-site string coupling from silently breaking if the value ever changes.
pub(crate) const RELOAD_SENTINEL: &str = "__reload__";

/// Handle a Telegram bot command (`/start`, `/help`, `/status`, `/reload`).
///
/// Returns `Some(response_text)` if the command is a known bot command,
/// or `None` if the message should be routed to the agent as regular text.
/// Only the first whitespace-delimited token is matched; trailing arguments are ignored
/// (e.g. `/start deep_link_payload` still matches `/start`).
///
/// Note: `/reload` returns `RELOAD_SENTINEL` — the actual reload logic
/// (session + spill clearing) is performed in `dispatch_update` where I/O access
/// is available. Here we only signal that the command was recognized.
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
             /status — Show bot status\n\
             /reload — Remove stale tool history, keep conversation\n\n\
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
        // /reload is recognized here; dispatch_update intercepts RELOAD_SENTINEL
        // to call execute_reload() with the I/O context (session + spill stores).
        "/reload" => Some(RELOAD_SENTINEL.to_string()),
        _ => None,
    }
}

/// Re-export execute_reload from the application layer for use in dispatch_update.
pub use crate::application::reload::execute_reload;

// Re-export shared credential resolution functions for backward compatibility.
pub use crate::interface::shared::{check_provider_readiness, resolve_api_key};

/// Re-export session key helper for BDD tests.
#[cfg(any(test, feature = "test-support"))]
pub use services::session_key_for_source;
