//! Exercises `dispatch_command`/`handle_*` routing with an in-memory ctx and sink writer.
use super::{
    dispatch_command, dispatch_ext_command, handle_abort, handle_clear_history, handle_new_session,
    handle_resume_session, handle_rewind_to, handle_steer, persist_current_session,
};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::message::Message;
use crate::domain::session::{ContextSpillStore, Session, SessionStore, SpillEntry, SpillIndex};
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::interface::cli::protocol::{AgentCommand, ToolRegistration};
use crate::interface::cli::uds::DispatchCtx;
use crate::interface::cli::uds_cancel::{CancelHandle, CancelSlot};
use crate::interface::cli::uds_ext_protocol::{ClientToolRegistry, new_client_tool_registry};
use crate::interface::cli::uds_session::AgentSession;
use std::sync::Arc;
#[derive(Debug, Default)]
pub(super) struct RecordingSpillStore {
    pub(super) cleared: std::sync::Mutex<Vec<String>>,
}
impl ContextSpillStore for RecordingSpillStore {
    fn append(
        &self,
        _session_key: &str,
        _entry: &SpillEntry,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), crate::domain::error::DomainError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async { Ok(()) })
    }
    fn recall(
        &self,
        _session_key: &str,
        _id: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Option<SpillEntry>, crate::domain::error::DomainError>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async { Ok(None) })
    }
    fn list_entries(
        &self,
        _session_key: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Arc<Vec<SpillIndex>>, crate::domain::error::DomainError>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async { Ok(Arc::new(Vec::new())) })
    }
    fn clear(
        &self,
        session_key: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), crate::domain::error::DomainError>>
                + Send
                + '_,
        >,
    > {
        self.cleared.lock().unwrap().push(session_key.to_string());
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn recording_spill_store_default_has_entries_is_false() {
    assert!(
        !RecordingSpillStore::default()
            .has_entries("s")
            .await
            .unwrap()
    );
}
#[derive(Debug, Default)]
pub(super) struct SessionAwareTool {
    pub(super) seen: std::sync::Mutex<Vec<String>>,
}

impl Tool for SessionAwareTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "session_aware".into(),
            description: "test".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }

    fn set_session_key(&self, session_key: String) {
        self.seen.lock().unwrap().push(session_key);
    }

    fn execute(
        &self,
        _arguments: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<ToolResult, crate::domain::error::DomainError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async {
            Ok(ToolResult {
                content: String::new(),
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}

fn make_agent() -> AgentLoopImpl {
    make_agent_with(
        Box::new(crate::infrastructure::tools::registry::ToolRegistryImpl::new()),
        None,
    )
}

pub(super) fn make_agent_with(
    tool_registry: Box<dyn crate::domain::tool::ToolRegistry>,
    spill_store: Option<std::sync::Arc<dyn ContextSpillStore>>,
) -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider: crate::interface::test_support::make_stub_provider(),
        tool_registry,
        model: "stub".into(),
        max_tokens: 100,
        temperature: 0.0,
        spill_store,
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
    })
}

/// Owns everything a `DispatchCtx` borrows so individual tests stay short.
pub(super) struct Fixture {
    pub(super) agent: AgentLoopImpl,
    pub(super) messages: Vec<Message>,
    pub(super) session: AgentSession,
    pub(super) session_key: String,
    pub(super) store: FileSessionStore,
    _tmp: tempfile::TempDir,
    writer: tokio::io::Sink,
    pub(super) cancel: CancelHandle,
    registry: ClientToolRegistry,
    ephemeral: bool,
    pub(super) last_persisted_message_index: usize,
    /// Injected system prompt mirrored into `DispatchCtx::system_prompt`.
    /// Default `""` keeps existing callers free of live/durable skew.
    pub(super) system_prompt: String,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = FileSessionStore::new(tmp.path());
        Self {
            agent: make_agent(),
            messages: Vec::new(),
            session: AgentSession::new("stub".into(), "cli:test".into()),
            session_key: "cli:test".to_string(),
            store,
            _tmp: tmp,
            writer: tokio::io::sink(),
            cancel: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
            registry: new_client_tool_registry(),
            ephemeral: false,
            last_persisted_message_index: 0,
            system_prompt: String::new(),
        }
    }

    pub(super) fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = prompt.into();
        self
    }

    pub(super) fn ctx(&mut self) -> DispatchCtx<'_> {
        let initial_stats = crate::interface::cli::uds_session::compute_session_stats(
            &self.session_key,
            &self.messages,
        );
        DispatchCtx {
            execution_state: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
            wire_mode: crate::interface::cli::uds_wire::ConnectionWireMode::legacy(),
            base_dir: self._tmp.path(),
            agent: &mut self.agent,
            messages: &mut self.messages,
            conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::interface::cli::uds_snapshots::ConversationSnapshotData::default(),
            )),
            state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                self.session.state_snapshot(0, None, 0, None),
            )),
            session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
            tool_catalogue_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session: &mut self.session,
            stdout: Some(&mut self.writer),
            session_key: &mut self.session_key,
            session_store: &self.store,
            ephemeral: self.ephemeral,
            system_prompt: self.system_prompt.as_str(),
            cancel_handle: self.cancel.clone(),
            turn_control: std::sync::Arc::default(),
            broadcast_tx: None,
            _ext_registry: None,
            client_tool_registry: self.registry.clone(),
            current_client_id: 0,
            subagent_registry: None,
            notification_rx: None,
            workflow_state: None,
            workflow_config: None,
            provider_reload: None,
            provider_reload_inputs: None,
            last_persisted_message_index: self.last_persisted_message_index,
            durable_prefix_dirty: false,
        }
    }
}

pub(super) fn tool_reg(name: &str) -> ToolRegistration {
    ToolRegistration {
        name: name.into(),
        description: "desc".into(),
        parameters_schema: r#"{"type":"object"}"#.into(),
        stable_id: None,
    }
}

#[tokio::test]
async fn steer_runs_immediately_when_idle() {
    // Idle steer is acted on now (drained), not left queued — this lets a
    // post-cancel steer actually execute instead of being stranded (#896).
    let mut fx = Fixture::new();
    {
        let mut ctx = fx.ctx();
        assert!(!handle_steer(&mut ctx, None, "steer", "hello".into()).await);
    }
    assert!(fx.session.drain_pending().is_empty(), "idle steer ran");
    assert!(
        fx.messages
            .iter()
            .any(|m| m.role == crate::domain::message::Role::User && m.content == "hello"),
        "the steered instruction should have been run"
    );
}

#[tokio::test]
async fn steer_prepends_when_streaming() {
    let mut fx = Fixture::new();
    fx.session.set_streaming(true);
    {
        let mut ctx = fx.ctx();
        assert!(!handle_steer(&mut ctx, Some("s"), "steer", "hi".into()).await);
    }
    assert_eq!(fx.session.drain_pending().len(), 1);
}

#[tokio::test]
async fn abort_returns_false() {
    let mut fx = Fixture::new();
    let mut ctx = fx.ctx();
    assert!(!handle_abort(&mut ctx, Some("a"), "abort").await);
}

#[tokio::test]
async fn clear_history_blocked_while_streaming() {
    let mut fx = Fixture::new();
    fx.session.set_streaming(true);
    let mut ctx = fx.ctx();
    assert!(!handle_clear_history(&mut ctx, Some("c"), "clear_history").await);
}

#[tokio::test]
async fn clear_history_clears_when_idle() {
    let mut fx = Fixture::new();
    fx.messages.push(Message::user("hi"));
    {
        let mut ctx = fx.ctx();
        assert!(!handle_clear_history(&mut ctx, None, "clear_history").await);
    }
    assert!(fx.messages.is_empty());
}

#[tokio::test]
async fn rewind_blocked_while_streaming() {
    let mut fx = Fixture::new();
    fx.session.set_streaming(true);
    let mut ctx = fx.ctx();
    assert!(!handle_rewind_to(&mut ctx, Some("r"), "rewind_to", Some(0), None).await);
}

#[tokio::test]
async fn rewind_invalid_target() {
    let mut fx = Fixture::new();
    let mut ctx = fx.ctx();
    assert!(!handle_rewind_to(&mut ctx, Some("r"), "rewind_to", Some(999), None).await);
}

#[tokio::test]
async fn rewind_valid_truncates_and_persists() {
    let mut fx = Fixture::new();
    fx.messages.push(Message::user("first"));
    fx.messages.push(Message::assistant("answer", vec![]));
    {
        let mut ctx = fx.ctx();
        assert!(!handle_rewind_to(&mut ctx, None, "rewind_to", Some(0), None).await);
    }
    assert!(fx.messages.is_empty());
}

#[tokio::test]
async fn rewind_valid_clears_spill_store_for_current_key() {
    let spill = std::sync::Arc::new(RecordingSpillStore::default());
    let mut fx = Fixture::new();
    fx.agent = make_agent_with(
        Box::new(crate::infrastructure::tools::registry::ToolRegistryImpl::new()),
        Some(spill.clone()),
    );
    fx.messages.push(Message::user("first"));
    fx.messages.push(Message::assistant("answer", vec![]));
    {
        let mut ctx = fx.ctx();
        assert!(!handle_rewind_to(&mut ctx, None, "rewind_to", Some(0), None).await);
    }

    assert_eq!(
        spill.cleared.lock().unwrap().as_slice(),
        &["cli:test".to_string()]
    );
}

// #1061 blocker: rewind-by-messageId resolution is unit-tested directly on
// `resolve_rewind_target` in uds_progress_clear_tests (a paged client's window
// index is never valid against the full conversation).

#[tokio::test]
async fn new_session_uses_fresh_key_and_clears_old_messages() {
    let mut fx = Fixture::new();
    fx.messages.push(Message::user("old turn"));
    let old_key = fx.session_key.clone();
    fx.store.claim(&old_key).unwrap();
    {
        let mut ctx = fx.ctx();
        assert!(!handle_new_session(&mut ctx, None, "new_session").await);
    }

    assert!(fx.messages.is_empty());
    assert_ne!(fx.session_key, old_key);
    assert!(fx.session_key.starts_with("chat-"));
    FileSessionStore::new(fx._tmp.path())
        .claim(&old_key)
        .expect("/new_session must release the old session ownership lock");
}

#[tokio::test]
async fn new_session_updates_tools_and_clears_new_spill_key() {
    let tool = std::sync::Arc::new(SessionAwareTool::default());
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    registry.register(tool.clone());
    let spill = std::sync::Arc::new(RecordingSpillStore::default());
    let mut fx = Fixture::new();
    fx.agent = make_agent_with(Box::new(registry), Some(spill.clone()));
    {
        let mut ctx = fx.ctx();
        assert!(!handle_new_session(&mut ctx, None, "new_session").await);
    }

    let tool_keys = tool.seen.lock().unwrap();
    assert_eq!(tool_keys.as_slice(), &[fx.session_key.clone()]);
    let cleared = spill.cleared.lock().unwrap();
    assert_eq!(cleared.as_slice(), &[fx.session_key.clone()]);
}

#[tokio::test]
async fn resume_blocked_while_streaming() {
    let mut fx = Fixture::new();
    fx.session.set_streaming(true);
    let mut ctx = fx.ctx();
    assert!(!handle_resume_session(&mut ctx, Some("rs"), "resume_session", "other".into()).await);
}

#[tokio::test]
async fn resume_blocked_in_ephemeral() {
    let mut fx = Fixture::new();
    fx.ephemeral = true;
    let mut ctx = fx.ctx();
    assert!(!handle_resume_session(&mut ctx, None, "resume_session", "other".into()).await);
}

#[tokio::test]
async fn resume_rejects_invalid_name() {
    let mut fx = Fixture::new();
    let mut ctx = fx.ctx();
    assert!(!handle_resume_session(&mut ctx, None, "resume_session", "bad name!".into()).await);
}

#[tokio::test]
async fn resume_session_not_found() {
    let mut fx = Fixture::new();
    let mut ctx = fx.ctx();
    assert!(!handle_resume_session(&mut ctx, None, "resume_session", "missing".into()).await);
}

#[tokio::test]
async fn failed_resume_releases_target_claim() {
    let mut fx = Fixture::new();
    {
        let mut ctx = fx.ctx();
        assert!(!handle_resume_session(&mut ctx, None, "resume_session", "missing".into()).await);
    }

    let competing_store = FileSessionStore::new(fx._tmp.path());
    competing_store
        .claim(&Session::build_key("cli", "missing"))
        .expect("failed resume must not retain ownership of the missing target");
}

#[tokio::test]
async fn resume_session_success_loads_messages() {
    let mut fx = Fixture::new();
    // Pre-save a target session into the store.
    let key = Session::build_key("cli", "saved");
    let saved = Session {
        key: key.clone(),
        messages: vec![Message::user("restored")],
        workflow_run: None,
    };
    fx.store.save(&saved).await.unwrap();
    {
        let mut ctx = fx.ctx();
        assert!(
            !handle_resume_session(&mut ctx, Some("rs"), "resume_session", "saved".into()).await
        );
    }
    assert_eq!(fx.session_key, key);
    assert_eq!(fx.messages.len(), 1);
}

#[tokio::test]
async fn resume_updates_session_aware_tools() {
    let tool = std::sync::Arc::new(SessionAwareTool::default());
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    registry.register(tool.clone());
    let mut fx = Fixture::new();
    fx.agent = make_agent_with(Box::new(registry), None);
    let key = Session::build_key("cli", "saved");
    fx.store
        .save(&Session {
            key: key.clone(),
            messages: vec![Message::user("restored")],
            workflow_run: None,
        })
        .await
        .unwrap();
    {
        let mut ctx = fx.ctx();
        assert!(
            !handle_resume_session(&mut ctx, Some("rs"), "resume_session", "saved".into()).await
        );
    }

    let tool_keys = tool.seen.lock().unwrap();
    assert_eq!(tool_keys.as_slice(), &[key]);
}

#[tokio::test]
async fn resume_loads_chat_session_by_full_key() {
    // Regression: /resume must load a user chat by its full `chat-…` key directly.
    let mut fx = Fixture::new();
    let key = "chat-1750000000-abc".to_string();
    let saved = Session {
        key: key.clone(),
        messages: vec![Message::user("restored chat")],
        workflow_run: None,
    };
    fx.store.save(&saved).await.unwrap();
    {
        let mut ctx = fx.ctx();
        assert!(!handle_resume_session(&mut ctx, Some("rs"), "resume_session", key.clone()).await);
    }
    assert_eq!(fx.session_key, key);
    assert_eq!(fx.messages.len(), 1);
}

#[tokio::test]
async fn persist_noop_when_ephemeral() {
    let mut fx = Fixture::new();
    fx.ephemeral = true;
    let mut ctx = fx.ctx();
    assert!(persist_current_session(&mut ctx).await.is_ok());
}

#[tokio::test]
async fn persist_noop_when_session_key_empty() {
    let mut fx = Fixture::new();
    fx.session_key = String::new();
    let mut ctx = fx.ctx();
    assert!(persist_current_session(&mut ctx).await.is_ok());
}

#[tokio::test]
async fn persist_saves_when_durable() {
    let mut fx = Fixture::new();
    fx.messages.push(Message::user("keep"));
    let mut ctx = fx.ctx();
    assert!(persist_current_session(&mut ctx).await.is_ok());
}

#[tokio::test]
async fn dispatch_routes_abort() {
    let mut fx = Fixture::new();
    let cmd = AgentCommand::Abort {
        id: Some("a".into()),
    };
    let mut ctx = fx.ctx();
    assert!(!dispatch_command(cmd, &mut ctx).await);
}

#[tokio::test]
async fn dispatch_routes_steer() {
    let mut fx = Fixture::new();
    let cmd = AgentCommand::Steer {
        id: None,
        message: "go".into(),
    };
    let mut ctx = fx.ctx();
    assert!(!dispatch_command(cmd, &mut ctx).await);
}

#[tokio::test]
async fn dispatch_routes_prompt_runs_agent() {
    let mut fx = Fixture::new();
    let cmd = AgentCommand::Prompt {
        id: Some("p".into()),
        message: "hello".into(),
        streaming_behavior: None,
    };
    let mut ctx = fx.ctx();
    assert!(!dispatch_command(cmd, &mut ctx).await);
}

#[tokio::test]
async fn prompt_persists_session_after_turn() {
    // Regression: a completed prompt turn must persist the session immediately.
    let mut fx = Fixture::new();
    let cmd = AgentCommand::Prompt {
        id: Some("p".into()),
        message: "remember this".into(),
        streaming_behavior: None,
    };
    {
        let mut ctx = fx.ctx();
        assert!(!dispatch_command(cmd, &mut ctx).await);
    }
    let loaded = fx.store.load("cli:test").await.unwrap();
    assert!(
        loaded.is_some(),
        "a completed prompt turn should have persisted the session"
    );
    assert!(
        !loaded.unwrap().messages.is_empty(),
        "persisted session should contain the turn's messages"
    );
}

#[tokio::test]
async fn dispatch_routes_follow_up_runs_agent() {
    let mut fx = Fixture::new();
    let cmd = AgentCommand::FollowUp {
        id: None,
        message: "more".into(),
    };
    let mut ctx = fx.ctx();
    assert!(!dispatch_command(cmd, &mut ctx).await);
}

#[tokio::test]
async fn dispatch_routes_rewind_to() {
    let mut fx = Fixture::new();
    let cmd = AgentCommand::RewindTo {
        id: Some("r".into()),
        message_index: Some(5),
        message_id: None,
    };
    let mut ctx = fx.ctx();
    assert!(!dispatch_command(cmd, &mut ctx).await);
}

#[tokio::test]
async fn dispatch_routes_set_model_invalid() {
    let mut fx = Fixture::new();
    let cmd = AgentCommand::SetModel {
        id: None,
        model: None,
        provider: None,
        model_id: None,
    };
    let mut ctx = fx.ctx();
    assert!(!dispatch_command(cmd, &mut ctx).await);
}

#[tokio::test]
async fn dispatch_routes_set_model_valid() {
    let mut fx = Fixture::new();
    let cmd = AgentCommand::SetModel {
        id: Some("m".into()),
        model: Some("openai/gpt-5.2".into()),
        provider: None,
        model_id: None,
    };
    {
        let mut ctx = fx.ctx();
        assert!(!dispatch_command(cmd, &mut ctx).await);
    }
    assert_eq!(fx.agent.model(), "openai/gpt-5.2");
}

#[tokio::test]
async fn dispatch_routes_set_workflow_automation_inactive() {
    let mut fx = Fixture::new();
    let cmd = AgentCommand::SetWorkflowAutomation {
        id: None,
        auto_continue: Some(true),
        completion_nudge: None,
    };
    let mut ctx = fx.ctx();
    // workflow_config is None → "workflow is not active" error branch.
    assert!(!dispatch_command(cmd, &mut ctx).await);
}

#[tokio::test]
async fn dispatch_routes_resume_session_ephemeral() {
    let mut fx = Fixture::new();
    fx.ephemeral = true;
    let cmd = AgentCommand::ResumeSession {
        id: None,
        session: "x".into(),
    };
    let mut ctx = fx.ctx();
    assert!(!dispatch_command(cmd, &mut ctx).await);
}

#[tokio::test]
async fn dispatch_routes_fieldless_get_state() {
    let mut fx = Fixture::new();
    let cmd = AgentCommand::GetState {
        agent_id: None,
        id: Some("g".into()),
    };
    let mut ctx = fx.ctx();
    // Handled by the fieldless fast-path; returns early.
    assert!(!dispatch_command(cmd, &mut ctx).await);
}

#[tokio::test]
async fn dispatch_register_then_unregister_tools() {
    let mut fx = Fixture::new();
    let reg_cmd = AgentCommand::RegisterTools {
        id: Some("rt".into()),
        tools: vec![tool_reg("weather")],
    };
    {
        let mut ctx = fx.ctx();
        assert!(!dispatch_command(reg_cmd, &mut ctx).await);
    }
    // The extension tool should now appear in the agent registry.
    assert!(
        fx.agent
            .runtime_tool_names()
            .contains(&"weather".to_string())
    );

    let unreg_cmd = AgentCommand::UnregisterTools {
        id: Some("ut".into()),
        tools: vec!["weather".into()],
    };
    {
        let mut ctx = fx.ctx();
        assert!(!dispatch_command(unreg_cmd, &mut ctx).await);
    }
    assert!(fx.agent.runtime_tool_names().is_empty());
}

#[tokio::test]
async fn dispatch_tool_result_unknown_is_noop() {
    let mut fx = Fixture::new();
    let cmd = AgentCommand::ToolResult {
        tool_call_id: "no-such".into(),
        content: "data".into(),
        is_error: false,
    };
    let mut ctx = fx.ctx();
    assert!(!dispatch_command(cmd, &mut ctx).await);
}

#[tokio::test]
async fn dispatch_ext_command_unregister_unknown_noop() {
    let mut fx = Fixture::new();
    let cmd = AgentCommand::UnregisterTools {
        id: None,
        tools: vec!["never-registered".into()],
    };
    let mut ctx = fx.ctx();
    assert!(!dispatch_ext_command(cmd, &mut ctx, None, "unregister_tools").await);
}
