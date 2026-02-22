// Background services: inbound processor, outbound dispatcher, health, heartbeat, cron.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::application::cron_executor;
use crate::application::heartbeat;
use crate::domain::agent::AgentLoop;
use crate::domain::channel::{Channel, ChannelTarget};
use crate::domain::message::{Message, Role};
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::bus::{InboundMessage, OutboundMessage};
use crate::infrastructure::config::HealthConfig;
use crate::infrastructure::health::server::{HealthServer, StaticReadiness};
use crate::infrastructure::persistence::cron_store::FileCronStore;
use crate::infrastructure::persistence::workspace_store::FileHeartbeatTaskSource;

use super::Gateway;

impl Gateway {
    /// Inbound message processing task.
    pub(super) async fn run_inbound_processor(
        mut inbound_rx: mpsc::Receiver<InboundMessage>,
        agent: Arc<dyn AgentLoop>,
        session_store: Arc<dyn SessionStore>,
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
        session_store: &Arc<dyn SessionStore>,
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
        agent: &Arc<dyn AgentLoop>,
        session_store: &Arc<dyn SessionStore>,
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
        agent: Arc<dyn AgentLoop>,
        workspace: PathBuf,
    ) {
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
            match heartbeat::execute_heartbeat_tick(&source, &*agent, timeout).await {
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
    pub(super) async fn run_cron_tick(
        store: Arc<FileCronStore>,
        agent: Arc<dyn AgentLoop>,
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
