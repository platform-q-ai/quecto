// Gateway service: starts channels, heartbeat, cron, and health server.

use std::path::PathBuf;
use std::sync::Arc;

use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::cron_executor;
use crate::application::heartbeat;
use crate::domain::agent::AgentLoop;
use crate::domain::message::{Message, Role};
use crate::domain::provider::LlmProvider;
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::auth::credential_store::{Credential, CredentialStore};
use crate::infrastructure::bus::{InboundMessage, MessageBus, OutboundMessage};
use crate::infrastructure::channels::telegram::{TelegramChannel, TelegramUpdate};
use crate::infrastructure::config::{Config, HealthConfig};
use crate::infrastructure::health::server::{HealthServer, StaticReadiness};
use crate::infrastructure::persistence::cron_store::FileCronStore;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::providers;
use crate::infrastructure::providers::fallback::FallbackProvider;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::cron_tool::CronTool;
use crate::infrastructure::tools::message::MessageTool;
use crate::infrastructure::tools::registry::ToolRegistryImpl;
use crate::infrastructure::tools::spawn::SpawnTool;
use crate::infrastructure::tools::web_search::WebSearchTool;
use crate::infrastructure::voice::groq_whisper::GroqWhisperClient;

use tokio::sync::mpsc;

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
struct EventLoopContext {
    inbound_tx: mpsc::Sender<InboundMessage>,
    inbound_rx: mpsc::Receiver<InboundMessage>,
    outbound_tx: mpsc::Sender<OutboundMessage>,
    outbound_rx: mpsc::Receiver<OutboundMessage>,
    agent: Arc<AgentLoopImpl>,
    session_store: Arc<FileSessionStore>,
    telegram: TelegramChannel,
    config: Config,
    workspace: PathBuf,
    cron_store: Arc<FileCronStore>,
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
            self.cron_store,
            self.agent.clone(),
            self.config.tools.cron.exec_timeout_minutes,
        ));

        // Core messaging pipeline — select until one stops or shutdown.
        tokio::select! {
            _ = Gateway::run_telegram_polling(
                self.telegram.clone(), self.inbound_tx, self.config,
            ) => {
                tracing::info!("Telegram polling stopped");
            }
            _ = Gateway::run_inbound_processor(
                self.inbound_rx, self.agent, self.session_store, self.outbound_tx,
            ) => {
                tracing::info!("Inbound processor stopped");
            }
            _ = Gateway::run_outbound_dispatcher(self.outbound_rx, self.telegram) => {
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

        let provider = Arc::new(self.build_fallback_provider()?);
        let cron_store = Arc::new(FileCronStore::new(&self.base_dir));
        let (agent, mut bus) = self.build_agent(workspace.clone(), provider, cron_store.clone());
        let agent = Arc::new(agent);

        let info = agent.info();
        let model = &self.config.agents.defaults.model;
        tracing::info!(tools = info.tool_count, model, "quecto gateway starting");

        let (inbound_tx, inbound_rx, outbound_tx, outbound_rx) = Self::take_channels(&mut bus);
        let ctx = EventLoopContext {
            inbound_tx,
            inbound_rx,
            outbound_tx,
            outbound_rx,
            agent,
            session_store: Arc::new(FileSessionStore::new(&self.base_dir)),
            telegram: TelegramChannel::new(&self.config.channels.telegram),
            config: self.config.clone(),
            workspace: workspace.clone(),
            cron_store,
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
    /// Loads the credential store snapshot once and uses it to resolve API keys
    /// for all providers, avoiding redundant file reads.
    fn build_fallback_provider(&self) -> Result<FallbackProvider, GatewayError> {
        let store = CredentialStore::new(&self.base_dir);
        let creds = store.load_snapshot().unwrap_or_default();

        let mut provider_list = Vec::new();
        self.maybe_add_provider(&mut provider_list, "openai", &creds);
        self.maybe_add_provider(&mut provider_list, "anthropic", &creds);
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
    ) {
        let (config_key, api_base) = match name {
            "openai" => (
                &self.config.providers.openai.api_key,
                &self.config.providers.openai.api_base,
            ),
            "anthropic" => (
                &self.config.providers.anthropic.api_key,
                &self.config.providers.anthropic.api_base,
            ),
            _ => return,
        };
        let api_key = resolve_api_key(config_key, creds, name);
        if api_key.is_empty() {
            return;
        }
        let base = if api_base.is_empty() {
            None
        } else {
            Some(api_base.clone())
        };
        if let Some(p) = providers::create_provider(name, api_key, base) {
            list.push(p);
        }
    }

    /// Build agent loop with tool registry and message bus.
    fn build_agent(
        &self,
        workspace: PathBuf,
        provider: Arc<FallbackProvider>,
        cron_store: Arc<FileCronStore>,
    ) -> (AgentLoopImpl, MessageBus) {
        let sandbox = Sandbox::new(
            Some(workspace.clone()),
            self.config.agents.defaults.restrict_to_workspace,
        );
        let mut registry = ToolRegistryImpl::with_core_tools(workspace, sandbox);
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

        let agent = AgentLoopImpl::new(AgentLoopConfig {
            provider,
            tool_registry: Box::new(registry),
            model: self.config.agents.defaults.model.clone(),
            max_tokens: self.config.agents.defaults.max_tokens,
            temperature: self.config.agents.defaults.temperature,
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

    /// Telegram long-polling task.
    async fn run_telegram_polling(
        telegram: TelegramChannel,
        inbound_tx: mpsc::Sender<InboundMessage>,
        config: Config,
    ) {
        if !telegram.is_enabled() {
            tracing::info!("Telegram disabled, polling not started");
            std::future::pending::<()>().await;
            return;
        }

        tracing::info!("Telegram polling started");
        let mut offset: i64 = 0;

        loop {
            let poll_result = Self::poll_once(&telegram, &inbound_tx, offset, &config).await;
            match poll_result {
                Ok(new_offset) => offset = new_offset,
                Err(()) => return,
            }
        }
    }

    /// Execute one poll cycle. Returns updated offset, or Err if channel closed.
    async fn poll_once(
        telegram: &TelegramChannel,
        inbound_tx: &mpsc::Sender<InboundMessage>,
        mut offset: i64,
        config: &Config,
    ) -> Result<i64, ()> {
        match telegram.get_updates(offset, 30).await {
            Ok(updates) => {
                for update in updates {
                    offset = update.update_id + 1;
                    Self::dispatch_update(telegram, &update, inbound_tx, config).await?;
                }
                Ok(offset)
            }
            Err(e) => {
                tracing::error!(error = %e, "Telegram getUpdates failed, retrying in 5s");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                Ok(offset)
            }
        }
    }

    /// Process a single Telegram update and dispatch it to the inbound channel.
    ///
    /// Known bot commands (`/start`, `/help`, `/status`) are handled directly
    /// and responded to via `telegram.send_message()` without going through
    /// the agent loop. Voice messages are transcribed via Groq Whisper before
    /// routing. Unknown messages (including unknown commands) are forwarded to
    /// the inbound channel for agent processing.
    ///
    /// Returns Err(()) if the inbound channel is closed.
    async fn dispatch_update(
        telegram: &TelegramChannel,
        update: &TelegramUpdate,
        inbound_tx: &mpsc::Sender<InboundMessage>,
        config: &Config,
    ) -> Result<(), ()> {
        // Try text message first.
        if let Some(msg) = TelegramChannel::parse_update(update) {
            if !telegram.is_user_allowed(&msg.sender_id) {
                tracing::warn!(sender_id = msg.sender_id, "unauthorized Telegram user");
                return Ok(());
            }

            // Check for bot commands before routing to agent.
            if let Some(response) = handle_bot_command(&msg.text, config) {
                if let Err(e) = telegram.send_message(&msg.chat_id, &response).await {
                    tracing::error!(error = %e, "failed to send bot command response");
                }
                return Ok(());
            }

            let inbound = InboundMessage {
                source: format!("telegram:{}", msg.chat_id),
                sender_id: msg.sender_id,
                text: msg.text,
            };
            return inbound_tx.send(inbound).await.map_err(|_| {
                tracing::error!("inbound channel closed");
            });
        }

        // Try voice message.
        if let Some((sender_id, chat_id, file_id)) = TelegramChannel::parse_voice_update(update) {
            if !telegram.is_user_allowed(&sender_id) {
                tracing::warn!(sender_id = sender_id, "unauthorized Telegram user");
                return Ok(());
            }

            let text =
                Self::handle_voice_message(telegram, &chat_id, &file_id, &config.voice).await;

            let Some(transcribed_text) = text else {
                return Ok(());
            };

            let inbound = InboundMessage {
                source: format!("telegram:{}", chat_id),
                sender_id,
                text: transcribed_text,
            };
            return inbound_tx.send(inbound).await.map_err(|_| {
                tracing::error!("inbound channel closed");
            });
        }

        Ok(())
    }

    /// Download voice audio from Telegram's file API.
    ///
    /// Returns the raw audio bytes, or sends an error to the user and returns `None`.
    async fn download_voice_audio(
        telegram: &TelegramChannel,
        chat_id: &str,
        file_id: &str,
    ) -> Option<Vec<u8>> {
        let file_path = match telegram.get_file(file_id).await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(error = %e, "failed to get voice file info");
                let _ = telegram
                    .send_message(chat_id, "Sorry, I could not process your voice message.")
                    .await;
                return None;
            }
        };

        match telegram.download_file(&file_path).await {
            Ok(b) => Some(b),
            Err(e) => {
                tracing::error!(error = %e, "failed to download voice file");
                let _ = telegram
                    .send_message(chat_id, "Sorry, I could not process your voice message.")
                    .await;
                None
            }
        }
    }

    /// Handle a voice message: download, transcribe, and return the text.
    ///
    /// Returns `None` if an error occurred (error message already sent to user).
    async fn handle_voice_message(
        telegram: &TelegramChannel,
        chat_id: &str,
        file_id: &str,
        voice_config: &crate::infrastructure::config::VoiceConfig,
    ) -> Option<String> {
        if voice_config.groq.api_key.is_empty() {
            let _ = telegram
                .send_message(chat_id, "Sorry, voice transcription is not configured.")
                .await;
            return None;
        }

        let audio_bytes = Self::download_voice_audio(telegram, chat_id, file_id).await?;

        let whisper = Self::build_whisper_client(voice_config);
        match whisper.transcribe_bytes(audio_bytes, "voice.ogg").await {
            Ok(result) => Some(result.text),
            Err(e) => {
                tracing::error!(error = %e, "voice transcription failed");
                let _ = telegram
                    .send_message(chat_id, "Sorry, I could not transcribe your voice message.")
                    .await;
                None
            }
        }
    }

    /// Build a Groq Whisper client from voice configuration.
    fn build_whisper_client(
        voice_config: &crate::infrastructure::config::VoiceConfig,
    ) -> GroqWhisperClient {
        if voice_config.groq.api_base.is_empty() {
            GroqWhisperClient::new(&voice_config.groq.api_key)
        } else {
            GroqWhisperClient::with_base_url(
                &voice_config.groq.api_key,
                &voice_config.groq.api_base,
            )
        }
    }

    /// Inbound message processing task.
    async fn run_inbound_processor(
        mut inbound_rx: mpsc::Receiver<InboundMessage>,
        agent: Arc<AgentLoopImpl>,
        session_store: Arc<FileSessionStore>,
        outbound_tx: mpsc::Sender<OutboundMessage>,
    ) {
        while let Some(msg) = inbound_rx.recv().await {
            let mut messages = Self::load_session(&session_store, &msg).await;

            messages.push(Message {
                role: Role::User,
                content: msg.text.clone(),
                tool_calls: vec![],
                tool_call_id: None,
            });

            let response_text =
                Self::process_and_save(&agent, &session_store, &msg, &mut messages).await;

            let outbound = OutboundMessage {
                target: msg.source.clone(),
                text: response_text,
            };
            if outbound_tx.send(outbound).await.is_err() {
                tracing::error!("outbound channel closed");
                return;
            }
        }
    }

    /// Load session messages for an inbound message, or return empty vec.
    async fn load_session(
        session_store: &Arc<FileSessionStore>,
        msg: &InboundMessage,
    ) -> Vec<Message> {
        let session_key = Session::build_key("telegram", &msg.source);
        match session_store.load(&session_key).await {
            Ok(Some(session)) => session.messages,
            Ok(None) => Vec::new(),
            Err(e) => {
                tracing::error!(error = %e, key = session_key, "failed to load session");
                Vec::new()
            }
        }
    }

    /// Process messages through agent loop, save session, return response text.
    async fn process_and_save(
        agent: &Arc<AgentLoopImpl>,
        session_store: &Arc<FileSessionStore>,
        msg: &InboundMessage,
        messages: &mut Vec<Message>,
    ) -> String {
        let session_key = Session::build_key("telegram", &msg.source);
        match agent.process(messages).await {
            Ok(result) => {
                let session = Session {
                    key: session_key,
                    messages: messages.clone(),
                };
                if let Err(e) = session_store.save(&session).await {
                    tracing::error!(error = %e, "failed to save session");
                }
                result.response
            }
            Err(e) => {
                tracing::error!(error = %e, "agent processing failed");
                format!("Error: {}", e)
            }
        }
    }

    /// Outbound message dispatching task.
    async fn run_outbound_dispatcher(
        mut outbound_rx: mpsc::Receiver<OutboundMessage>,
        telegram: TelegramChannel,
    ) {
        while let Some(msg) = outbound_rx.recv().await {
            // Parse target: "telegram:chat_id"
            if let Some(chat_id) = msg.target.strip_prefix("telegram:") {
                if let Err(e) = telegram.send_message(chat_id, &msg.text).await {
                    tracing::error!(
                        error = %e,
                        chat_id = chat_id,
                        "failed to send Telegram message"
                    );
                }
            } else {
                tracing::warn!(target_str = msg.target, "unknown outbound target");
            }
        }
    }

    /// Health server task.
    ///
    /// Starts the health HTTP server if enabled in configuration.
    /// If disabled, suspends forever (does not consume a select! slot).
    async fn run_health_server(config: HealthConfig) {
        if !config.enabled {
            tracing::info!("Health server disabled");
            std::future::pending::<()>().await;
            return;
        }

        let addr = format!("127.0.0.1:{}", config.port);
        // Readiness starts as true (at least one provider exists per build_fallback_provider).
        // NOTE: This is static — /ready reflects startup state only. Dynamic readiness
        // (wired to FallbackProvider cooldown) is planned for a future PR.
        let readiness = Arc::new(StaticReadiness::new(true));
        match HealthServer::bind(&addr, readiness).await {
            Ok(server) => {
                tracing::info!(port = config.port, "health server started");
                server.run().await;
            }
            Err(e) => {
                tracing::error!(error = %e, addr = addr, "failed to bind health server");
                // Don't crash the gateway — just log and suspend.
                std::future::pending::<()>().await;
            }
        }
    }

    /// Heartbeat timer task.
    ///
    /// Periodically loads HEARTBEAT.md from the workspace, parses tasks,
    /// and dispatches them through the agent. If disabled in config,
    /// suspends forever (does not consume a select! slot).
    async fn run_heartbeat(
        config: crate::infrastructure::config::HeartbeatConfig,
        agent: Arc<AgentLoopImpl>,
        workspace: PathBuf,
    ) {
        if !config.enabled {
            tracing::info!("Heartbeat disabled");
            std::future::pending::<()>().await;
            return;
        }

        let interval_secs = u64::from(config.interval);
        let interval = std::time::Duration::from_secs(interval_secs);
        // Timeout per task: interval minus 10s margin, clamped to [30s, 300s].
        let timeout_secs = interval_secs.saturating_sub(10).clamp(30, 300);
        let timeout = std::time::Duration::from_secs(timeout_secs);
        tracing::info!(
            interval_secs = config.interval,
            timeout_secs = timeout_secs,
            "heartbeat timer started"
        );

        loop {
            tokio::time::sleep(interval).await;
            tracing::debug!("heartbeat tick");
            match heartbeat::execute_heartbeat_tick(&workspace, &*agent, timeout).await {
                Ok(results) => {
                    for result in &results {
                        tracing::info!(
                            task = result.message.as_str(),
                            via_spawn = result.dispatched_via_spawn,
                            "heartbeat task completed"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "heartbeat tick failed");
                }
            }
        }
    }

    /// Cron tick timer task.
    ///
    /// Periodically checks for due cron jobs and dispatches them through the agent.
    /// Uses a short check interval (2s) so jobs fire promptly.
    async fn run_cron_tick(
        store: Arc<FileCronStore>,
        agent: Arc<AgentLoopImpl>,
        timeout_minutes: u32,
    ) {
        let check_interval = std::time::Duration::from_secs(2);
        let timeout = std::time::Duration::from_secs(u64::from(timeout_minutes) * 60);
        tracing::info!(
            check_interval_secs = 2,
            timeout_minutes = timeout_minutes,
            "cron tick timer started"
        );

        loop {
            tokio::time::sleep(check_interval).await;
            tracing::debug!("cron tick");
            match cron_executor::execute_cron_tick(&*store, &*agent, timeout).await {
                Ok(results) => {
                    for result in &results {
                        tracing::info!(
                            job_id = result.job_id.as_str(),
                            ok = result.ok,
                            "cron job executed"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "cron tick failed");
                }
            }
        }
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

/// Resolve an API key for a provider from a credential snapshot.
///
/// The credential store snapshot takes priority over the config-file key.
/// Expired credentials are ignored (falls back to config key).
/// Operates on a pre-loaded snapshot to avoid redundant file I/O.
pub fn resolve_api_key(
    config_key: &str,
    creds: &std::collections::HashMap<String, Credential>,
    provider: &str,
) -> String {
    if let Some(cred) = creds.get(provider) {
        if !cred.is_expired() {
            return cred.token.clone();
        }
    }
    config_key.to_string()
}

/// Check which providers have expired credentials and need re-authentication.
///
/// Operates on a pre-loaded snapshot to avoid redundant file I/O.
pub fn check_provider_readiness(
    creds: &std::collections::HashMap<String, Credential>,
) -> Vec<String> {
    creds
        .values()
        .filter(|c| c.is_expired())
        .map(|c| c.provider.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::config::Config;

    #[test]
    fn test_gateway_creation() {
        let config: Config = serde_json::from_str("{}").unwrap();
        let gw = Gateway::new(config, PathBuf::from("/tmp/quecto-test"));
        assert_eq!(gw.base_dir, PathBuf::from("/tmp/quecto-test"));
    }

    #[test]
    fn test_resolve_workspace_default() {
        let config: Config = serde_json::from_str(
            r#"{
            "agents": { "defaults": { "workspace": "/opt/workspace" } }
        }"#,
        )
        .unwrap();
        let gw = Gateway::new(config, PathBuf::from("/tmp/test"));
        let ws = gw.resolve_workspace();
        assert_eq!(ws, PathBuf::from("/opt/workspace"));
    }

    #[tokio::test]
    async fn test_gateway_no_providers_error() {
        let config: Config = serde_json::from_str("{}").unwrap();
        let gw = Gateway::new(config, PathBuf::from("/tmp/quecto-test"));
        let result = gw.run().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("no LLM providers"),
            "expected NoProviders, got: {}",
            err
        );
    }

    #[test]
    fn test_resolve_api_key_from_credential_store() {
        use crate::infrastructure::auth::credential_store::{
            AuthMethod, Credential, CredentialStore,
        };
        let tmp = tempfile::TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());
        store
            .store(Credential {
                provider: "openai".to_string(),
                token: "sk-from-store".to_string(),
                method: AuthMethod::Token,
                expires_at: None,
            })
            .unwrap();

        let creds = store.load_snapshot().unwrap();
        let config: Config = serde_json::from_str("{}").unwrap();
        let resolved = resolve_api_key(&config.providers.openai.api_key, &creds, "openai");
        assert_eq!(resolved, "sk-from-store");
    }

    #[test]
    fn test_resolve_api_key_prefers_store_over_config() {
        use crate::infrastructure::auth::credential_store::{
            AuthMethod, Credential, CredentialStore,
        };
        let tmp = tempfile::TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());
        store
            .store(Credential {
                provider: "openai".to_string(),
                token: "sk-from-store".to_string(),
                method: AuthMethod::Token,
                expires_at: None,
            })
            .unwrap();

        let creds = store.load_snapshot().unwrap();
        let config: Config =
            serde_json::from_str(r#"{"providers": {"openai": {"api_key": "sk-from-config"}}}"#)
                .unwrap();
        let resolved = resolve_api_key(&config.providers.openai.api_key, &creds, "openai");
        assert_eq!(resolved, "sk-from-store");
    }

    #[test]
    fn test_resolve_api_key_falls_back_to_config() {
        use crate::infrastructure::auth::credential_store::CredentialStore;
        let tmp = tempfile::TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());
        // No credential stored

        let creds = store.load_snapshot().unwrap();
        let config: Config =
            serde_json::from_str(r#"{"providers": {"openai": {"api_key": "sk-from-config"}}}"#)
                .unwrap();
        let resolved = resolve_api_key(&config.providers.openai.api_key, &creds, "openai");
        assert_eq!(resolved, "sk-from-config");
    }

    #[test]
    fn test_resolve_api_key_ignores_expired_credential() {
        use crate::infrastructure::auth::credential_store::{
            AuthMethod, Credential, CredentialStore,
        };
        let tmp = tempfile::TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());
        store
            .store(Credential {
                provider: "openai".to_string(),
                token: "sk-expired".to_string(),
                method: AuthMethod::Token,
                expires_at: Some(0), // always expired
            })
            .unwrap();

        let creds = store.load_snapshot().unwrap();
        let config: Config =
            serde_json::from_str(r#"{"providers": {"openai": {"api_key": "sk-from-config"}}}"#)
                .unwrap();
        let resolved = resolve_api_key(&config.providers.openai.api_key, &creds, "openai");
        assert_eq!(resolved, "sk-from-config");
    }

    #[test]
    fn test_check_provider_readiness_reports_expired() {
        use crate::infrastructure::auth::credential_store::{
            AuthMethod, Credential, CredentialStore,
        };
        let tmp = tempfile::TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());
        store
            .store(Credential {
                provider: "openai".to_string(),
                token: "sk-expired".to_string(),
                method: AuthMethod::Token,
                expires_at: Some(0),
            })
            .unwrap();

        let creds = store.load_snapshot().unwrap();
        let needs_reauth = check_provider_readiness(&creds);
        assert!(needs_reauth.contains(&"openai".to_string()));
    }

    // --- Bot command tests ---

    #[test]
    fn test_handle_bot_command_start() {
        let config: Config = serde_json::from_str("{}").unwrap();
        let result = handle_bot_command("/start", &config);
        assert!(result.is_some(), "/start should be handled");
        let text = result.unwrap();
        assert!(
            text.contains("quecto"),
            "start response should mention quecto"
        );
        assert!(
            text.contains("Welcome"),
            "start response should be welcoming"
        );
    }

    #[test]
    fn test_handle_bot_command_help() {
        let config: Config = serde_json::from_str("{}").unwrap();
        let result = handle_bot_command("/help", &config);
        assert!(result.is_some(), "/help should be handled");
        let text = result.unwrap();
        assert!(text.contains("/start"), "help should list /start");
        assert!(text.contains("/help"), "help should list /help");
        assert!(text.contains("/status"), "help should list /status");
    }

    #[test]
    fn test_handle_bot_command_status() {
        let config: Config =
            serde_json::from_str(r#"{"agents": {"defaults": {"model": "gpt-5.2"}}}"#).unwrap();
        let result = handle_bot_command("/status", &config);
        assert!(result.is_some(), "/status should be handled");
        let text = result.unwrap();
        assert!(text.contains("Model:"), "status should show model");
        assert!(text.contains("gpt-5.2"), "status should show model name");
    }

    #[test]
    fn test_handle_bot_command_unknown_returns_none() {
        let config: Config = serde_json::from_str("{}").unwrap();
        let result = handle_bot_command("/unknown", &config);
        assert!(result.is_none(), "/unknown should not be handled");
    }

    #[test]
    fn test_handle_bot_command_regular_text_returns_none() {
        let config: Config = serde_json::from_str("{}").unwrap();
        let result = handle_bot_command("Hello, how are you?", &config);
        assert!(result.is_none(), "regular text should not be handled");
    }

    #[test]
    fn test_handle_bot_command_start_with_args() {
        let config: Config = serde_json::from_str("{}").unwrap();
        // /start with args (deep link) should still be handled
        let result = handle_bot_command("/start ref123", &config);
        assert!(result.is_some(), "/start with args should be handled");
    }

    #[test]
    fn test_check_provider_readiness_active_is_empty() {
        use crate::infrastructure::auth::credential_store::{
            AuthMethod, Credential, CredentialStore,
        };
        let tmp = tempfile::TempDir::new().unwrap();
        let store = CredentialStore::new(tmp.path());
        store
            .store(Credential {
                provider: "openai".to_string(),
                token: "sk-active".to_string(),
                method: AuthMethod::Token,
                expires_at: None,
            })
            .unwrap();

        let creds = store.load_snapshot().unwrap();
        let needs_reauth = check_provider_readiness(&creds);
        assert!(needs_reauth.is_empty());
    }

    #[tokio::test]
    async fn test_run_health_server_starts_and_responds() {
        use crate::infrastructure::config::HealthConfig;

        // Bind to port 0 to get a random available port
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind to random port");
        let port = listener.local_addr().unwrap().port();
        drop(listener); // Release port so health server can bind to it

        let config = HealthConfig {
            enabled: true,
            port,
        };

        // Spawn health server in background
        let handle = tokio::spawn(Gateway::run_health_server(config));

        // Wait briefly for the server to start
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Make a request
        let client = reqwest::Client::new();
        let resp = client
            .get(format!("http://127.0.0.1:{}/health", port))
            .send()
            .await
            .expect("request should succeed");
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["status"], "ok");

        // Ready should return true (gateway sets readiness to true)
        let resp = client
            .get(format!("http://127.0.0.1:{}/ready", port))
            .send()
            .await
            .expect("request should succeed");
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["ready"], true);

        handle.abort();
    }

    #[tokio::test]
    async fn test_run_health_server_disabled_suspends() {
        use crate::infrastructure::config::HealthConfig;

        let config = HealthConfig {
            enabled: false,
            port: 0,
        };

        // Should not return — just suspend forever. We verify by racing with a timeout.
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            Gateway::run_health_server(config),
        )
        .await;

        assert!(
            result.is_err(),
            "disabled health server should suspend (timeout expected)"
        );
    }
}
