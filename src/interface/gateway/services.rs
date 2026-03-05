// Background services: inbound processor, outbound dispatcher, health, heartbeat, cron.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::cron_executor;
use crate::application::heartbeat;
use crate::domain::agent::{AgentInfo, AgentLoop, AgentResult};
use crate::domain::channel::{Channel, ChannelTarget};
use crate::domain::error::DomainError;
use crate::domain::message::{Message, Role};
use crate::domain::provider::LlmProvider;
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::bus::{InboundMessage, OutboundMessage};
use crate::infrastructure::config::{Config, HealthConfig};
use crate::infrastructure::health::server::{HealthServer, StaticReadiness};
use crate::infrastructure::logging::redact_api_keys;
use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
use crate::infrastructure::persistence::cron_store::FileCronStore;
use crate::infrastructure::persistence::workspace_store::FileHeartbeatTaskSource;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::cron_tool::CronTool;
use crate::infrastructure::tools::message::MessageTool;
use crate::infrastructure::tools::recall::RecallTool;
use crate::infrastructure::tools::registry::ToolRegistryImpl;
use crate::infrastructure::tools::spawn::SpawnTool;
use crate::infrastructure::tools::web_search::WebSearchTool;

use super::Gateway;

/// Wrapper that injects a transient system prompt before each `process()` call,
/// building a fresh prompt (datetime + skills) each time so the agent always
/// knows the current date/time. Mirrors the REPL/CLI system prompt pattern.
struct SystemPromptAgent {
    inner: Arc<dyn AgentLoop>,
    skill_prompt: String,
}

impl AgentLoop for SystemPromptAgent {
    fn process<'a>(
        &'a self,
        messages: &'a mut Vec<Message>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<AgentResult, DomainError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let prompt = crate::interface::shared::build_system_prompt(&self.skill_prompt, &None);
            messages.insert(0, Message::system(prompt.clone()));
            let result = self.inner.process(messages).await;
            // Remove the transient system prompt by position (index 0) rather than
            // content equality — avoids accidental removal of user system messages
            // that happen to match the prompt content.
            if messages
                .first()
                .map(|m| m.role == Role::System && m.content == prompt)
                .unwrap_or(false)
            {
                messages.remove(0);
            }
            result
        })
    }

    fn info(&self) -> AgentInfo {
        self.inner.info()
    }
}

/// Parameters for the cron tick timer task.
pub(super) struct CronTickContext {
    pub(super) store: Arc<FileCronStore>,
    pub(super) agent: Arc<dyn AgentLoop>,
    pub(super) timeout_minutes: u32,
    pub(super) skill_prompt: String,
    pub(super) outbound_tx: mpsc::Sender<OutboundMessage>,
    /// Fallback delivery target when a cron job has no `deliver_to` set.
    /// Comes from `config.channels.telegram.default_send_to`.
    pub(super) default_send_to: Option<String>,
}

pub(super) struct InboundProcessorContext {
    pub(super) agent: Arc<dyn AgentLoop>,
    pub(super) agent_builder: Option<Arc<InboundAgentBuilder>>,
    pub(super) session_store: Arc<dyn SessionStore>,
    pub(super) outbound_tx: mpsc::Sender<OutboundMessage>,
    pub(super) max_session_messages: usize,
    /// Skill prompt loaded at startup, combined with datetime per-request.
    pub(super) skill_prompt: String,
}

pub(super) struct InboundAgentBuilder {
    pub(super) config: Config,
    pub(super) base_dir: PathBuf,
    pub(super) provider: Arc<dyn LlmProvider>,
    pub(super) cron_store: Arc<FileCronStore>,
}

impl InboundAgentBuilder {
    pub(super) fn build(
        &self,
        session_key: &str,
        outbound_tx: mpsc::Sender<OutboundMessage>,
    ) -> AgentLoopImpl {
        let workspace = PathBuf::from(self.config.workspace_path());
        let sandbox = Sandbox::new(
            Some(workspace.clone()),
            self.config.agents.defaults.restrict_to_workspace,
        );
        let exec_settings = ToolRegistryImpl::exec_registry_settings_from_config(&self.config);
        let mut registry =
            ToolRegistryImpl::with_core_tools_and_exec_settings(workspace, sandbox, exec_settings);

        let default_send_to = self.config.channels.telegram.default_send_to.clone();
        registry.register(Arc::new(MessageTool::new(outbound_tx, default_send_to)));
        let brave = &self.config.tools.web.brave;
        let brave_api_key = if brave.enabled && !brave.api_key.is_empty() {
            Some(brave.api_key.clone())
        } else {
            None
        };
        registry.register(Arc::new(WebSearchTool::new(brave_api_key)));
        registry.register(Arc::new(CronTool::new(self.cron_store.clone())));
        registry.register(Arc::new(SpawnTool::with_base_dir(
            vec![],
            self.config.agents.defaults.restrict_to_workspace,
            self.base_dir.clone(),
        )));

        let spill_store = Arc::new(FileContextSpillStore::new(self.base_dir.clone()));
        registry.register(Arc::new(RecallTool::new(
            spill_store.clone(),
            session_key.to_string(),
        )));

        AgentLoopImpl::new(AgentLoopConfig {
            provider: self.provider.clone(),
            tool_registry: Box::new(registry),
            model: self.config.agents.defaults.model.clone(),
            max_tokens: self.config.agents.defaults.max_tokens,
            temperature: self.config.agents.defaults.temperature,
            spill_store: Some(spill_store),
            session_key: session_key.to_string(),
            context_collapse_after_turns: self.config.agents.defaults.context_collapse_after_turns,
            max_context_tokens: self.config.agents.defaults.max_context_tokens,
            progress_callback: None,
            streaming: false,
        })
        .with_max_tool_iterations(self.config.agents.defaults.max_tool_iterations)
    }
}

impl Gateway {
    /// Inbound message processing task.
    pub(super) async fn run_inbound_processor(
        mut inbound_rx: mpsc::Receiver<InboundMessage>,
        ctx: InboundProcessorContext,
    ) {
        while let Some(msg) = inbound_rx.recv().await {
            let mut messages = Self::load_session(&ctx.session_store, &msg).await;

            messages.push(Message::user(msg.text.clone()));

            trim_session_messages(&mut messages, ctx.max_session_messages);

            let response_text = Self::process_and_save(&ctx, &msg, &mut messages).await;

            let outbound = OutboundMessage {
                target: msg.source.clone(),
                text: response_text,
            };
            if ctx.outbound_tx.send(outbound).await.is_err() {
                tracing::error!("outbound channel closed");
                return;
            }
        }
    }

    /// Load session messages for an inbound message, or return empty vec.
    ///
    /// The `source` field of `InboundMessage` is already in `channel:id` form
    /// (e.g. `"telegram:12345"`), so it is used directly as the session key.
    /// Do NOT prefix it again with `Session::build_key("telegram", ...)` —
    /// that would produce `"telegram:telegram:12345"` and break `/reload`
    /// which looks up `"telegram:<chat_id>"`.
    async fn load_session(
        session_store: &Arc<dyn SessionStore>,
        msg: &InboundMessage,
    ) -> Vec<Message> {
        let session_key = msg.source.clone();
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
    ///
    /// Uses `SystemPromptAgent` to inject a transient datetime+skills system
    /// prompt for the duration of the call — it is automatically stripped
    /// after processing so it is never persisted in session history.
    async fn process_and_save(
        ctx: &InboundProcessorContext,
        msg: &InboundMessage,
        messages: &mut Vec<Message>,
    ) -> String {
        // source is already "channel:id" (e.g. "telegram:12345") — use directly.
        let session_key = msg.source.clone();
        let result = if let Some(ref builder) = ctx.agent_builder {
            let inner: Arc<dyn AgentLoop> =
                Arc::new(builder.build(&session_key, ctx.outbound_tx.clone()));
            let agent = SystemPromptAgent {
                inner,
                skill_prompt: ctx.skill_prompt.clone(),
            };
            agent.process(messages).await
        } else {
            // Shared agent path (no per-session builder) — wrap with system prompt.
            let agent = SystemPromptAgent {
                inner: ctx.agent.clone(),
                skill_prompt: ctx.skill_prompt.clone(),
            };
            agent.process(messages).await
        };
        match result {
            Ok(result) => {
                trim_session_messages(messages, ctx.max_session_messages);
                let session = Session {
                    key: session_key,
                    messages: messages.clone(),
                };
                if let Err(e) = ctx.session_store.save(&session).await {
                    tracing::error!(error = %e, "failed to save session");
                }
                result.response
            }
            Err(e) => {
                tracing::error!(error = %e, "agent processing failed");
                format!("Error: {}", redact_api_keys(&e.to_string()))
            }
        }
    }

    /// Outbound message dispatching task.
    pub(super) async fn run_outbound_dispatcher(
        mut outbound_rx: mpsc::Receiver<OutboundMessage>,
        channel: Arc<dyn Channel>,
    ) {
        while let Some(msg) = outbound_rx.recv().await {
            let target = ChannelTarget::parse(&msg.target);
            if let ChannelTarget::Unsupported(raw) = &target {
                tracing::warn!(target = raw, "unknown outbound target");
                continue;
            }

            if let Err(e) = channel.send_message(&target, &msg.text).await {
                tracing::error!(error = %e, target = msg.target, "failed outbound send");
            }
        }
    }

    /// Health server task.
    ///
    /// Starts the health HTTP server if enabled in configuration.
    /// If disabled, suspends forever (does not consume a select! slot).
    pub(super) async fn run_health_server(config: HealthConfig) {
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
    pub(super) async fn run_heartbeat(
        config: crate::infrastructure::config::HeartbeatConfig,
        inner_agent: Arc<dyn AgentLoop>,
        workspace: PathBuf,
        skill_prompt: String,
    ) {
        let agent = SystemPromptAgent {
            inner: inner_agent,
            skill_prompt,
        };
        if !config.enabled {
            tracing::info!("Heartbeat disabled");
            std::future::pending::<()>().await;
            return;
        }

        let interval_secs = u64::from(config.interval);
        let interval = std::time::Duration::from_secs(interval_secs);
        let source = FileHeartbeatTaskSource::new(workspace);
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
            match heartbeat::execute_heartbeat_tick(&source, &agent, timeout).await {
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
    /// When a job has a `deliver_to` target and produces a successful response,
    /// the result is sent to the outbound channel for delivery (issue #106).
    pub(super) async fn run_cron_tick(ctx: CronTickContext) {
        let agent = SystemPromptAgent {
            inner: ctx.agent,
            skill_prompt: ctx.skill_prompt,
        };
        let store = ctx.store;
        let timeout_minutes = ctx.timeout_minutes;
        let outbound_tx = ctx.outbound_tx;
        let default_send_to = ctx.default_send_to;
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
            match cron_executor::execute_cron_tick(&*store, &agent, timeout).await {
                Ok(results) => {
                    for result in &results {
                        tracing::info!(
                            job_id = result.job_id.as_str(),
                            ok = result.ok,
                            "cron job executed"
                        );
                        deliver_cron_result(result, &outbound_tx, default_send_to.as_deref()).await;
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "cron tick failed");
                }
            }
        }
    }
}

/// Session key for an inbound message source (test-support only).
///
/// `source` is already `"channel:id"` — returned as-is. Must stay in sync
/// with `load_session` / `process_and_save`. Any future transformation must
/// be reflected here so BDD tests track the real contract.
#[cfg(any(test, feature = "test-support"))]
pub fn session_key_for_source(source: &str) -> String {
    source.to_string()
}

/// Deliver a successful cron result: job `deliver_to` → config `default_send_to` → drop.
async fn deliver_cron_result(
    result: &crate::domain::cron::CronJobResult,
    outbound_tx: &mpsc::Sender<OutboundMessage>,
    default_send_to: Option<&str>,
) {
    if !result.ok {
        return;
    }
    let target = result.deliver_to.as_deref().or(default_send_to);
    let Some(target) = target else {
        return;
    };
    let msg = OutboundMessage {
        target: target.to_string(),
        text: result.response.clone(),
    };
    if let Err(e) = outbound_tx.send(msg).await {
        tracing::error!(
            job_id = result.job_id.as_str(),
            target = target,
            error = %e,
            "failed to deliver cron result"
        );
    }
}

fn trim_session_messages(messages: &mut Vec<Message>, max_non_system_messages: usize) {
    if max_non_system_messages == 0 {
        messages.retain(|m| matches!(m.role, Role::System));
        return;
    }

    let non_system_count = messages
        .iter()
        .filter(|m| !matches!(m.role, Role::System))
        .count();
    if non_system_count <= max_non_system_messages {
        return;
    }

    let mut kept_non_system = 0usize;
    let mut start = messages.len();
    for i in (0..messages.len()).rev() {
        if !matches!(messages[i].role, Role::System) {
            kept_non_system += 1;
        }
        if kept_non_system > max_non_system_messages {
            start = i + 1;
            break;
        }
        start = i;
    }

    while start > 0 && matches!(messages[start].role, Role::Tool) {
        start -= 1;
    }

    let mut trimmed = Vec::with_capacity(messages.len());
    for (idx, message) in messages.drain(..).enumerate() {
        if matches!(message.role, Role::System) || idx >= start {
            trimmed.push(message);
        }
    }
    *messages = trimmed;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::agent::{AgentInfo, AgentLoop, AgentResult};
    use crate::domain::error::DomainError;
    use crate::domain::session::{Session, SessionStore};

    /// Agent that always returns a fixed text response without modifying messages.
    #[derive(Debug)]
    struct EchoAgent;

    impl AgentLoop for EchoAgent {
        fn process<'a>(
            &'a self,
            _messages: &'a mut Vec<Message>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<AgentResult, DomainError>> + Send + 'a>,
        > {
            Box::pin(async { Ok(crate::domain::agent::AgentResult::text("echo reply")) })
        }

        fn info(&self) -> AgentInfo {
            AgentInfo {
                tool_count: 0,
                skill_count: 0,
            }
        }
    }

    #[derive(Debug)]
    struct FailingAgent;

    impl AgentLoop for FailingAgent {
        fn process<'a>(
            &'a self,
            _messages: &'a mut Vec<Message>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<AgentResult, DomainError>> + Send + 'a>,
        > {
            Box::pin(async {
                Err(DomainError::Provider(
                    "upstream rejected key sk-secret-key-12345".to_string(),
                ))
            })
        }

        fn info(&self) -> AgentInfo {
            AgentInfo {
                tool_count: 0,
                skill_count: 0,
            }
        }
    }

    /// Session store that records the key used in the most recent save() call.
    #[derive(Debug, Default)]
    struct CapturingSessionStore {
        saved_key: std::sync::Mutex<Option<String>>,
    }

    impl SessionStore for CapturingSessionStore {
        fn load(
            &self,
            _key: &str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Option<Session>, DomainError>> + Send + '_>,
        > {
            Box::pin(async { Ok(None) })
        }

        fn save(
            &self,
            session: &Session,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + '_>>
        {
            *self.saved_key.lock().unwrap() = Some(session.key.clone());
            Box::pin(async { Ok(()) })
        }

        fn exists(
            &self,
            _key: &str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<bool, DomainError>> + Send + '_>,
        > {
            Box::pin(async { Ok(false) })
        }
    }

    #[derive(Debug, Default)]
    struct NoopSessionStore;

    impl SessionStore for NoopSessionStore {
        fn load(
            &self,
            _key: &str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Option<Session>, DomainError>> + Send + '_>,
        > {
            Box::pin(async { Ok(None) })
        }

        fn save(
            &self,
            _session: &Session,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + '_>>
        {
            Box::pin(async { Ok(()) })
        }

        fn exists(
            &self,
            _key: &str,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<bool, DomainError>> + Send + '_>,
        > {
            Box::pin(async { Ok(false) })
        }
    }

    #[test]
    fn test_trim_messages_keeps_latest_non_system() {
        let mut messages = vec![
            Message::user("u1"),
            Message::assistant("a1", vec![]),
            Message::user("u2"),
        ];

        trim_session_messages(&mut messages, 2);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "a1");
        assert_eq!(messages[1].content, "u2");
    }

    #[test]
    fn test_trim_messages_preserves_system_messages() {
        let mut messages = vec![
            Message::system("sys"),
            Message::user("u1"),
            Message::assistant("a1", vec![]),
        ];

        trim_session_messages(&mut messages, 1);

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, Role::System);
        assert_eq!(messages[1].content, "a1");
    }

    #[test]
    fn test_trim_messages_does_not_start_with_tool_message() {
        let mut messages = vec![
            Message::user("u1"),
            Message::assistant(
                "calling tool",
                vec![crate::domain::message::ToolCall {
                    id: "t1".to_string(),
                    name: "read".to_string(),
                    arguments: "{}".to_string(),
                }],
            ),
            Message::tool("t1", "tool result"),
            Message::user("u2"),
        ];

        trim_session_messages(&mut messages, 2);

        assert!(!messages.is_empty());
        assert_ne!(messages[0].role, Role::Tool);
        assert!(messages.iter().any(|m| m.role == Role::Tool));
    }

    #[tokio::test]
    async fn test_process_and_save_redacts_secret_in_error_response() {
        let ctx = InboundProcessorContext {
            agent: Arc::new(FailingAgent),
            agent_builder: None,
            session_store: Arc::new(NoopSessionStore),
            outbound_tx: tokio::sync::mpsc::channel(1).0,
            max_session_messages: 50,
            skill_prompt: String::new(),
        };

        let msg = InboundMessage {
            source: "telegram:12345".to_string(),
            sender_id: "12345".to_string(),
            text: "hello".to_string(),
        };
        let mut messages = vec![];

        let response = Gateway::process_and_save(&ctx, &msg, &mut messages).await;

        assert!(response.starts_with("Error:"));
        assert!(!response.contains("sk-secret-key-12345"));
        assert!(response.contains("sk-***"));
    }

    /// The inbound message source is already in "channel:id" form
    /// (e.g. "telegram:12345"). The session key must NOT add another
    /// "telegram:" prefix — the saved key must equal the source exactly.
    #[tokio::test]
    async fn test_session_key_matches_inbound_source_exactly() {
        let capturing_store = Arc::new(CapturingSessionStore::default());

        let ctx = InboundProcessorContext {
            agent: Arc::new(EchoAgent),
            agent_builder: None,
            session_store: capturing_store.clone(),
            outbound_tx: tokio::sync::mpsc::channel(1).0,
            max_session_messages: 50,
            skill_prompt: String::new(),
        };

        let msg = InboundMessage {
            source: "telegram:12345".to_string(),
            sender_id: "12345".to_string(),
            text: "hello".to_string(),
        };
        let mut messages = vec![];

        Gateway::process_and_save(&ctx, &msg, &mut messages).await;

        let saved_key = capturing_store
            .saved_key
            .lock()
            .unwrap()
            .clone()
            .expect("session should have been saved");

        assert_eq!(
            saved_key, "telegram:12345",
            "session key must not double-prefix source: expected 'telegram:12345', got '{}'",
            saved_key
        );
    }

    /// /reload uses Session::build_key("telegram", chat_id) → "telegram:<chat_id>".
    /// The inbound processor must save under the same key so /reload can find it.
    /// This test verifies the keys are consistent by writing with the inbound
    /// processor's key and reading back with the /reload key.
    #[tokio::test]
    async fn test_reload_key_matches_inbound_processor_key() {
        use crate::infrastructure::persistence::session_store::FileSessionStore;

        let td = tempfile::TempDir::new().expect("tempdir");
        let store = Arc::new(FileSessionStore::new(td.path()));

        // Simulate what the inbound processor saves:
        // source = "telegram:99999" → key must be "telegram:99999"
        let inbound_source = "telegram:99999";
        let inbound_key = inbound_source.to_string(); // correct: source IS the key

        let session = Session {
            key: inbound_key.clone(),
            messages: vec![Message::user("hi"), Message::assistant("hello", vec![])],
        };
        store.save(&session).await.expect("save");

        // /reload builds key as Session::build_key("telegram", chat_id)
        let reload_key = Session::build_key("telegram", "99999");

        assert_eq!(
            inbound_key, reload_key,
            "inbound processor key '{}' must match /reload key '{}'",
            inbound_key, reload_key
        );

        // Verify the session is actually readable via the reload key
        let loaded = store
            .load(&reload_key)
            .await
            .expect("load")
            .expect("session must exist");

        assert_eq!(loaded.messages.len(), 2);
    }
}
