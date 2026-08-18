//! Shared test environment for dispatch-level UDS tests.
//!
//! [`DispatchCtx`] borrows ~30 disjoint values, so every dispatch test used
//! to carry its own copy of the agent-config and context literals — three
//! divergent copies that had to be kept in sync by hand whenever the dispatch
//! path gained a field. This module is the single owner of those literals:
//! tests build a [`DispatchTestEnv`] (parameterised by provider and workflow)
//! and borrow a context from it.

use super::*;
use crate::domain::workflow::{WorkflowConfig, WorkflowEngine};
use crate::interface::shared::WorkflowStateHandle;

/// A fresh workflow engine handle with no template selected.
pub(super) fn make_workflow() -> WorkflowStateHandle {
    std::sync::Arc::new(std::sync::Mutex::new(
        WorkflowEngine::new(WorkflowConfig::default(), false).unwrap(),
    ))
}

/// A workflow engine handle with the (incomplete) `feature` template
/// selected, so workflow automation fires at the idle drain.
pub(super) fn make_selected_feature_workflow() -> WorkflowStateHandle {
    let workflow = make_workflow();
    workflow
        .lock()
        .unwrap()
        .select_template("feature", None)
        .unwrap();
    workflow
}

/// A workflow engine handle with the `feature` template selected and every
/// step checked, so only the completion nudge can fire at the idle drain.
pub(super) fn make_completed_feature_workflow() -> WorkflowStateHandle {
    let workflow = make_selected_feature_workflow();
    {
        let mut engine = workflow.lock().unwrap();
        let steps = engine.progress().total;
        for step in 1..=steps {
            engine.check(step).unwrap();
        }
    }
    workflow
}

/// The canonical agent-loop config for dispatch tests, parameterised by
/// provider so scripted providers slot in without copying the literal.
pub(super) fn make_dispatch_test_agent(
    provider: std::sync::Arc<dyn crate::domain::provider::LlmProvider>,
) -> crate::application::agent_loop::AgentLoopImpl {
    crate::application::agent_loop::AgentLoopImpl::new(
        crate::application::agent_loop::AgentLoopConfig {
            provider,
            tool_registry: Box::new(
                crate::infrastructure::tools::registry::ToolRegistryImpl::new(),
            ),
            model: "stub".into(),
            max_tokens: 100,
            temperature: 0.0,
            spill_store: None,
            session_key: "cli:test".into(),
            context_collapse_after_tool_calls: u32::MAX,
            max_context_tokens: 190_000,
            progress_callback: None,
            streaming: false,
            effort: None,
            audit_log: None,
            pin_recent_turns: 2,
            context_collapse_after_messages: u32::MAX,
            model_context_window: None,
            tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
        },
    )
}

/// Owns every value a [`DispatchCtx`] borrows so a single helper can build
/// the context inline (each field borrow is a disjoint field of this struct).
/// Fields are exposed so tests can pre-seed state (messages, pending queue,
/// control flags, session store) or swap the agent before borrowing a context.
pub(super) struct DispatchTestEnv {
    pub(super) tmp: tempfile::TempDir,
    pub(super) agent: crate::application::agent_loop::AgentLoopImpl,
    pub(super) messages: Vec<crate::domain::message::Message>,
    pub(super) session: AgentSession,
    pub(super) session_key: String,
    pub(super) store: crate::infrastructure::persistence::session_store::FileSessionStore,
    pub(super) writer: tokio::io::Sink,
    pub(super) workflow: WorkflowStateHandle,
    pub(super) turn_control: crate::interface::cli::uds_cancel::TurnControlHandle,
    /// Pre-seeded sub-agent notification channel; `ctx()` moves it into the
    /// context, mirroring the real dispatch loop's single owned receiver.
    pub(super) notification_rx:
        Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    pub(super) subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
}

impl DispatchTestEnv {
    /// Build an env around the given workflow handle and provider.
    pub(super) fn new(
        workflow: WorkflowStateHandle,
        provider: std::sync::Arc<dyn crate::domain::provider::LlmProvider>,
    ) -> Self {
        let tmp = tempfile::TempDir::new().unwrap();
        let store =
            crate::infrastructure::persistence::session_store::FileSessionStore::new(tmp.path());
        Self {
            tmp,
            agent: make_dispatch_test_agent(provider),
            messages: Vec::new(),
            session: AgentSession::new("stub".into(), "cli:test".into()),
            session_key: "cli:test".to_string(),
            store,
            writer: tokio::io::sink(),
            workflow,
            turn_control: std::sync::Arc::default(),
            notification_rx: None,
            subagent_registry: None,
        }
    }

    /// Env with a stub provider and a selected (incomplete) `feature`
    /// workflow, so the auto-continue nudge would normally fire at the idle
    /// boundary.
    pub(super) fn with_selected_feature() -> Self {
        Self::new(
            make_selected_feature_workflow(),
            crate::interface::test_support::make_stub_provider(),
        )
    }

    /// Env with a stub provider and an unselected workflow engine.
    pub(super) fn with_unselected_workflow() -> Self {
        Self::new(
            make_workflow(),
            crate::interface::test_support::make_stub_provider(),
        )
    }

    pub(super) fn ctx(&mut self) -> DispatchCtx<'_> {
        let initial_stats = crate::interface::cli::uds_session::compute_session_stats(
            &self.session_key,
            &self.messages,
        );
        let state = self.session.state_snapshot(0, None, 0, None);
        DispatchCtx {
            execution_state: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
            wire_mode: crate::interface::cli::uds_wire::ConnectionWireMode::legacy(),
            base_dir: self.tmp.path(),
            agent: &mut self.agent,
            messages: &mut self.messages,
            conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::interface::cli::uds_snapshots::ConversationSnapshotData::default(),
            )),
            state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(state)),
            session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
            tool_catalogue_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session: &mut self.session,
            stdout: Some(&mut self.writer),
            session_key: &mut self.session_key,
            session_store: &self.store,
            ephemeral: false,
            system_prompt: "",
            cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
            turn_control: self.turn_control.clone(),
            broadcast_tx: None,
            _ext_registry: None,
            client_tool_registry: crate::interface::cli::uds_ext_protocol::new_client_tool_registry(
            ),
            current_client_id: 0,
            subagent_registry: self.subagent_registry.clone(),
            notification_rx: self.notification_rx.take(),
            workflow_state: Some(self.workflow.clone()),
            workflow_config: Some(WorkflowConfig::default()),
            provider_reload: None,
            provider_reload_inputs: None,
            last_persisted_message_index: 0,
            durable_prefix_dirty: false,
        }
    }
}
