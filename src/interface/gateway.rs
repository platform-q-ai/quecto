// Gateway service: starts channels, heartbeat, cron, and health server.

use std::path::PathBuf;
use std::sync::Arc;

use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::agent::AgentLoop;
use crate::domain::message::{Message, Role};
use crate::domain::provider::LlmProvider;
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::bus::{InboundMessage, MessageBus, OutboundMessage};
use crate::infrastructure::channels::telegram::{TelegramChannel, TelegramUpdate};
use crate::infrastructure::config::Config;
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
}

impl EventLoopContext {
    /// Run the concurrent event loop until shutdown.
    async fn run(self) {
        let polling_telegram = self.telegram.clone();
        tokio::select! {
            _ = Gateway::run_telegram_polling(polling_telegram, self.inbound_tx) => {
                tracing::info!("Telegram polling stopped");
            }
            _ = Gateway::run_inbound_processor(
                self.inbound_rx, self.agent, self.session_store, self.outbound_tx
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
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .init();
        let workspace = self.resolve_workspace();
        let provider = Arc::new(self.build_fallback_provider()?);
        let (agent, mut bus) = self.build_agent(workspace, provider);
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
    fn build_fallback_provider(&self) -> Result<FallbackProvider, GatewayError> {
        let mut provider_list = Vec::new();
        self.maybe_add_provider(&mut provider_list, "openai");
        self.maybe_add_provider(&mut provider_list, "anthropic");
        if provider_list.is_empty() {
            return Err(GatewayError::NoProviders);
        }
        Ok(FallbackProvider::new(provider_list))
    }

    /// Try to create a provider and add it to the list.
    fn maybe_add_provider(&self, list: &mut Vec<Arc<dyn LlmProvider>>, name: &str) {
        let (api_key, api_base) = match name {
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
        if api_key.is_empty() {
            return;
        }
        let base = if api_base.is_empty() {
            None
        } else {
            Some(api_base.clone())
        };
        if let Some(p) = providers::create_provider(name, api_key.clone(), base) {
            list.push(p);
        }
    }

    /// Build agent loop with tool registry and message bus.
    fn build_agent(
        &self,
        workspace: PathBuf,
        provider: Arc<FallbackProvider>,
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
        let cron_store = Arc::new(FileCronStore::new(&self.base_dir));
        registry.register(Arc::new(CronTool::new(cron_store)));
        registry.register(Arc::new(SpawnTool::new(
            vec![],
            self.config.agents.defaults.restrict_to_workspace,
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
    ) {
        if !telegram.is_enabled() {
            tracing::info!("Telegram disabled, polling not started");
            std::future::pending::<()>().await;
            return;
        }

        tracing::info!("Telegram polling started");
        let mut offset: i64 = 0;

        loop {
            let poll_result = Self::poll_once(&telegram, &inbound_tx, offset).await;
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
    ) -> Result<i64, ()> {
        match telegram.get_updates(offset, 30).await {
            Ok(updates) => {
                for update in updates {
                    offset = update.update_id + 1;
                    Self::dispatch_update(telegram, &update, inbound_tx).await?;
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
    /// Returns Err(()) if the inbound channel is closed.
    async fn dispatch_update(
        telegram: &TelegramChannel,
        update: &TelegramUpdate,
        inbound_tx: &mpsc::Sender<InboundMessage>,
    ) -> Result<(), ()> {
        let Some(msg) = TelegramChannel::parse_update(update) else {
            return Ok(());
        };
        if !telegram.is_user_allowed(&msg.sender_id) {
            tracing::warn!(sender_id = msg.sender_id, "unauthorized Telegram user");
            return Ok(());
        }
        let inbound = InboundMessage {
            source: format!("telegram:{}", msg.chat_id),
            sender_id: msg.sender_id,
            text: msg.text,
        };
        inbound_tx.send(inbound).await.map_err(|_| {
            tracing::error!("inbound channel closed");
        })
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
}
