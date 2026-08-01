//! Unit tests for the runtime reasoning-effort protocol surface (#1067):
//! `set_effort` command parsing and the `effort` field of `get_state`.

use super::*;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::provider::EffortLevel;
use crate::interface::cli::protocol::AgentCommand;
use crate::interface::cli::uds_ext_protocol::new_client_tool_registry;

// ── protocol parsing ─────────────────────────────────────────────────────

#[test]
fn protocol_parses_set_effort_command() {
    let parsed: Result<AgentCommand, _> =
        serde_json::from_str(r#"{"type":"set_effort","id":"se-1","effort":"xhigh"}"#);
    let cmd = parsed.expect("set_effort must be a recognised agent-protocol command");
    assert_eq!(cmd.type_name(), "set_effort");
    assert_eq!(cmd.id(), Some("se-1"));
    // The payload must actually bind: a variant that drops or defaults the
    // `effort` field would satisfy the type/tag assertions alone.
    let AgentCommand::SetEffort { effort, .. } = cmd else {
        panic!("parsed command is not SetEffort");
    };
    assert_eq!(effort, "xhigh", "effort payload must be bound verbatim");
}

// ── get_state effort exposure ────────────────────────────────────────────

fn make_effort_test_agent(effort: Option<EffortLevel>) -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider: crate::interface::test_support::make_stub_provider(),
        tool_registry: Box::new(crate::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "stub".into(),
        max_tokens: 100,
        temperature: 0.0,
        spill_store: None,
        session_key: "cli:test".into(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    })
}

struct EffortFx {
    agent: AgentLoopImpl,
    messages: Vec<crate::domain::message::Message>,
    session: AgentSession,
    session_key: String,
    store: crate::infrastructure::persistence::session_store::FileSessionStore,
    _tmp: tempfile::TempDir,
    writer: tokio::io::Sink,
}

impl EffortFx {
    fn new(effort: Option<EffortLevel>) -> Self {
        let tmp = tempfile::TempDir::new().unwrap();
        Self {
            agent: make_effort_test_agent(effort),
            messages: Vec::new(),
            session: AgentSession::new("stub".into(), "cli:test".into()),
            session_key: "cli:test".into(),
            store: crate::infrastructure::persistence::session_store::FileSessionStore::new(
                tmp.path(),
            ),
            _tmp: tmp,
            writer: tokio::io::sink(),
        }
    }

    fn ctx(&mut self) -> DispatchCtx<'_> {
        let initial_stats = crate::interface::cli::uds_session::compute_session_stats(
            &self.session_key,
            &self.messages,
        );
        DispatchCtx {
            execution_state: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
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
            ephemeral: false,
            system_prompt: "",
            cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(
                crate::interface::cli::uds_cancel::CancelSlot::Idle,
            )),
            turn_control: std::sync::Arc::default(),
            broadcast_tx: None,
            _ext_registry: None,
            client_tool_registry: new_client_tool_registry(),
            current_client_id: 0,
            wire_mode: crate::interface::cli::uds_wire::ConnectionWireMode::legacy(),
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

#[test]
fn get_state_exposes_configured_effort() {
    let mut fx = EffortFx::new(Some(EffortLevel::High));
    let ctx = fx.ctx();
    let state = crate::interface::cli::uds_query::query_response_data(
        &AgentCommand::GetState { id: None },
        &ctx,
    )
    .expect("get_state must return data");
    assert_eq!(
        state["effort"], "high",
        "get_state must expose the effective effort level, got: {state}"
    );
}

#[test]
fn get_state_exposes_null_effort_when_unset() {
    let mut fx = EffortFx::new(None);
    let ctx = fx.ctx();
    let state = crate::interface::cli::uds_query::query_response_data(
        &AgentCommand::GetState { id: None },
        &ctx,
    )
    .expect("get_state must return data");
    assert!(
        state.as_object().map(|o| o.contains_key("effort")) == Some(true),
        "get_state must include an effort field (null when unset) so clients \
         can distinguish 'default' from 'missing capability', got: {state}"
    );
    assert!(
        state["effort"].is_null(),
        "unset effort must surface as null (provider default), got: {state}"
    );
}

#[test]
fn get_state_exposes_provider_effort_vocabulary() {
    // The agent is the single source of truth for the provider→levels rule:
    // get_state must carry the valid vocabulary so clients (TUI selector /
    // validation) never duplicate it (#1067 review).
    let mut fx = EffortFx::new(None);
    let ctx = fx.ctx();
    let state = crate::interface::cli::uds_query::query_response_data(
        &AgentCommand::GetState { id: None },
        &ctx,
    )
    .expect("get_state must return data");
    assert_eq!(
        state["effortLevels"],
        serde_json::json!(["none", "low", "medium", "high", "xhigh"]),
        "get_state must list the provider's valid effort levels, got: {state}"
    );
}

#[tokio::test]
async fn busy_snapshot_get_state_carries_effort_and_vocabulary() {
    // A TUI connecting mid-turn is served the frozen snapshot instead of the
    // live query; it must not silently drop the effort override or the
    // vocabulary (#1067 review: snapshot/live get_state shape parity).
    let mut fx = EffortFx::new(Some(EffortLevel::XHigh));
    let ctx = fx.ctx();
    crate::interface::cli::uds_snapshots::refresh_state_snapshot(&ctx).await;
    let snap = ctx.state_snapshot.read().await;
    let line = crate::interface::cli::uds_snapshots::build_get_state_line_live(&snap, &None, true);
    let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(
        value["data"]["effort"], "xhigh",
        "busy-connect snapshot get_state must carry the active effort, got: {value}"
    );
    assert_eq!(
        value["data"]["effortLevels"],
        serde_json::json!(["none", "low", "medium", "high", "xhigh"]),
        "busy-connect snapshot get_state must carry the vocabulary, got: {value}"
    );
}

// ── model switch resets effort ───────────────────────────────────────────

/// Switching models resets the session effort to `low`: a level chosen for
/// one provider (e.g. OpenAI `xhigh`) must not silently carry into another
/// provider's vocabulary, where it would be clamped while the UI still shows
/// the stale level. Explicit `low` on every switch is predictable and
/// cost-safe; the user re-raises effort deliberately via set_effort.
#[tokio::test]
async fn set_model_resets_the_session_effort_to_low() {
    let mut fx = EffortFx::new(Some(EffortLevel::XHigh));
    {
        let mut ctx = fx.ctx();
        let cmd: AgentCommand = serde_json::from_str(
            r#"{"type":"set_model","model":"anthropic-api/claude-sonnet-4-6"}"#,
        )
        .expect("set_model parses");
        crate::interface::cli::uds::uds_dispatch::dispatch_command(cmd, &mut ctx).await;
    }
    assert_eq!(
        fx.agent.effort(),
        Some(EffortLevel::Low),
        "a model switch must reset the session effort to low"
    );
}
