//! #926 (act half): once an idle parent is woken by a child completion, the
//! dispatch loop enqueues the one-line note and calls `drain_pending_and_nudge`
//! — which must drain the buffered note and RUN a parent turn so the note is
//! DELIVERED INTO the parent's conversation context, not merely left buffered.
//! The wake half is covered in `uds_multi_926_wake_tests`.
//!
//! Scope note: this asserts delivery-into-context against a stub provider — it
//! proves the woken parent runs a turn with the note in scope, NOT that the
//! note's (deliberately soft, #894) wording compels a real model to call
//! `get_messages` and integrate. The note-actionability half of AC2 (root
//! cause #2) is intentionally DEFERRED — see `SubagentNotification::to_message`
//! and the PR body; flipping the note to imperative would override the #894
//! anti-flood design and is left as an open design question.
use super::*;
use crate::domain::message::Role;

/// Owns every value a [`DispatchCtx`] borrows so the helper can build the
/// context inline (each field borrow is a disjoint field of this struct).
struct ActEnv {
    tmp: tempfile::TempDir,
    agent: crate::application::agent_loop::AgentLoopImpl,
    messages: Vec<crate::domain::message::Message>,
    session: AgentSession,
    session_key: String,
    store: crate::infrastructure::persistence::session_store::FileSessionStore,
    writer: tokio::io::Sink,
}

impl ActEnv {
    fn new() -> Self {
        let tmp = tempfile::TempDir::new().unwrap();
        let store =
            crate::infrastructure::persistence::session_store::FileSessionStore::new(tmp.path());
        Self {
            tmp,
            agent: crate::application::agent_loop::AgentLoopImpl::new(
                crate::application::agent_loop::AgentLoopConfig {
                    provider: crate::interface::test_support::make_stub_provider(),
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
                    system_prompt_provider: None,
                    audit_log: None,
                    pin_recent_turns: 2,
                    context_collapse_after_messages: u32::MAX,
                    model_context_window: None,
                },
            ),
            messages: Vec::new(),
            session: AgentSession::new("stub".into(), "cli:test".into()),
            session_key: "cli:test".to_string(),
            store,
            writer: tokio::io::sink(),
        }
    }

    fn ctx(&mut self) -> DispatchCtx<'_> {
        let initial_stats = crate::interface::cli::uds_session::compute_session_stats(
            &self.session_key,
            &self.messages,
        );
        let state = self.session.state_snapshot(0, None, 0, None);
        DispatchCtx {
            wire_mode: crate::interface::cli::uds_wire::ConnectionWireMode::legacy(),
            base_dir: self.tmp.path(),
            agent: &mut self.agent,
            messages: &mut self.messages,
            conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(state)),
            session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
            extension_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session: &mut self.session,
            stdout: Some(&mut self.writer),
            session_key: &mut self.session_key,
            session_store: &self.store,
            ephemeral: false,
            system_prompt: "",
            cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
            turn_control: std::sync::Arc::default(),
            broadcast_tx: None,
            ext_registry: None,
            client_tool_registry: crate::interface::cli::uds_ext_protocol::new_client_tool_registry(
            ),
            current_client_id: 0,
            subagent_registry: None,
            notification_rx: None,
            workflow_state: None,
            workflow_config: None,
            provider_reload: None,
            provider_reload_inputs: None,
            last_persisted_message_index: 0,
            durable_prefix_dirty: false,
        }
    }
}

/// A single new child completion (no `await`, so it is delivered passively) must
/// drive a parent turn that surfaces the note INTO the conversation context —
/// proving wake → delivery, not just buffering. After the drain the pending
/// queue is empty and the parent's message history carries the child's summary.
/// (This proves delivery-into-context only; whether the soft note then compels
/// integration is the deferred AC2 question — see the module doc.)
#[tokio::test]
async fn test_926_single_completion_drives_a_parent_turn() {
    let mut env = ActEnv::new();

    // Idle parent receives one fresh completion note (the dispatch loop enqueues
    // this just before calling drain_pending_and_nudge).
    let is_new = env.session.enqueue_subagent_notification(
        "researcher".to_string(),
        1,
        "[subagent] Agent 'researcher' completed. Last output: all tests pass".to_string(),
        true,
    );
    assert!(
        is_new,
        "first completion must be new (would drive delivery)"
    );

    let mut ctx = env.ctx();
    drain_pending_and_nudge(&mut ctx).await;

    assert_eq!(
        ctx.session
            .state_snapshot(0, None, 0, None)
            .pending_message_count,
        0,
        "the buffered completion note must be consumed by the idle drain"
    );
    let surfaced = ctx
        .messages
        .iter()
        .any(|m| m.role == Role::System && m.content.contains("researcher"));
    assert!(
        surfaced,
        "the completion note must be delivered into the parent's context, got: {:?}",
        ctx.messages
    );
}
