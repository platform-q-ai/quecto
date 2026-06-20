//! Region-coverage tests for the UDS dispatch routing + handlers.
//!
//! These exercise the pure routing/decision logic of `dispatch_command` and
//! the individual `handle_*` helpers using an in-memory `DispatchCtx` backed by
//! a stub provider and a `tokio::io::sink()` writer. No real socket is opened;
//! events are written to the sink (or dropped). The socket accept loop itself
//! is covered by BDD tests, not here.
use super::{
    dispatch_command, dispatch_ext_command, handle_abort, handle_clear_history,
    handle_resume_session, handle_rewind_to, handle_steer, persist_current_session,
};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::message::Message;
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::interface::cli::protocol::{AgentCommand, ToolRegistration};
use crate::interface::cli::uds::DispatchCtx;
use crate::interface::cli::uds_cancel::{CancelHandle, CancelSlot};
use crate::interface::cli::uds_ext_protocol::{ClientToolRegistry, new_client_tool_registry};
use crate::interface::cli::uds_session::AgentSession;

fn make_agent() -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider: crate::interface::test_support::make_stub_provider(),
        tool_registry: Box::new(crate::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "stub".into(),
        max_tokens: 100,
        temperature: 0.0,
        spill_store: None,
        session_key: "cli:test".into(),
        context_collapse_after_turns: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
    })
}

/// Owns everything a `DispatchCtx` borrows so individual tests stay short.
struct Fixture {
    agent: AgentLoopImpl,
    messages: Vec<Message>,
    session: AgentSession,
    session_key: String,
    store: FileSessionStore,
    _tmp: tempfile::TempDir,
    writer: tokio::io::Sink,
    cancel: CancelHandle,
    registry: ClientToolRegistry,
    ephemeral: bool,
}

impl Fixture {
    fn new() -> Self {
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
        }
    }

    fn ctx(&mut self) -> DispatchCtx<'_> {
        DispatchCtx {
            agent: &mut self.agent,
            messages: &mut self.messages,
            session: &mut self.session,
            stdout: &mut self.writer,
            session_key: &mut self.session_key,
            session_store: &self.store,
            ephemeral: self.ephemeral,
            system_prompt: "",
            cancel_handle: self.cancel.clone(),
            broadcast_tx: None,
            ext_registry: None,
            client_tool_registry: self.registry.clone(),
            current_client_id: 0,
            subagent_registry: None,
            notification_rx: None,
            workflow_state: None,
            workflow_config: None,
        }
    }
}

fn tool_reg(name: &str) -> ToolRegistration {
    ToolRegistration {
        name: name.into(),
        description: "desc".into(),
        parameters_schema: r#"{"type":"object"}"#.into(),
    }
}

// ─── handle_steer ────────────────────────────────────────────────────────────

#[tokio::test]
async fn steer_enqueues_when_idle() {
    let mut fx = Fixture::new();
    {
        let mut ctx = fx.ctx();
        assert!(!handle_steer(&mut ctx, None, "steer", "hello".into()).await);
    }
    assert_eq!(fx.session.drain_pending().len(), 1);
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

// ─── handle_abort ────────────────────────────────────────────────────────────

#[tokio::test]
async fn abort_returns_false() {
    let mut fx = Fixture::new();
    let mut ctx = fx.ctx();
    assert!(!handle_abort(&mut ctx, Some("a"), "abort").await);
}

// ─── handle_clear_history ────────────────────────────────────────────────────

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

// ─── handle_rewind_to ────────────────────────────────────────────────────────

#[tokio::test]
async fn rewind_blocked_while_streaming() {
    let mut fx = Fixture::new();
    fx.session.set_streaming(true);
    let mut ctx = fx.ctx();
    assert!(!handle_rewind_to(&mut ctx, Some("r"), "rewind_to", 0).await);
}

#[tokio::test]
async fn rewind_invalid_target() {
    let mut fx = Fixture::new();
    let mut ctx = fx.ctx();
    assert!(!handle_rewind_to(&mut ctx, Some("r"), "rewind_to", 999).await);
}

#[tokio::test]
async fn rewind_valid_truncates_and_persists() {
    let mut fx = Fixture::new();
    fx.messages.push(Message::user("first"));
    fx.messages.push(Message::assistant("answer", vec![]));
    {
        let mut ctx = fx.ctx();
        assert!(!handle_rewind_to(&mut ctx, None, "rewind_to", 0).await);
    }
    assert!(fx.messages.is_empty());
}

// ─── handle_resume_session ───────────────────────────────────────────────────

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

// ─── persist_current_session ─────────────────────────────────────────────────

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

// ─── dispatch_command routing ────────────────────────────────────────────────

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
        message_index: 5,
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
        id: Some("g".into()),
    };
    let mut ctx = fx.ctx();
    // Handled by the fieldless fast-path; returns early.
    assert!(!dispatch_command(cmd, &mut ctx).await);
}

// ─── extension command dispatch ──────────────────────────────────────────────

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
            .tool_registry_extension_names()
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
    assert!(fx.agent.tool_registry_extension_names().is_empty());
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
