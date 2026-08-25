//! #935: switching models via the UDS dispatch path must re-derive and apply
//! the new model's registry output cap. Self-contained (own minimal
//! `DispatchCtx`) so it stays independent of `uds_dispatch_cov_tests.rs` and
//! neither file crosses the source line-count gate.

use super::dispatch_command;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::interface::cli::protocol::AgentCommand;
use crate::interface::cli::uds::DispatchCtx;
use crate::interface::cli::uds_cancel::CancelSlot;
use crate::interface::cli::uds_ext_protocol::new_client_tool_registry;
use crate::interface::cli::uds_session::AgentSession;

#[tokio::test]
async fn dispatch_set_model_ignores_filesystem_registry_not_in_active_runtime() {
    // The agent is configured with max_tokens=100. A stale filesystem registry
    // advertises maxTokens=50, but the active last-good runtime provider has no
    // matching descriptor, so set_model must not independently reread/apply it.
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"qwen3p7-plus","maxTokens":50}]}}}"#,
    )
    .unwrap();

    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
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
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    });
    // Before the switch the stub model has no registry cap → configured value.
    assert_eq!(agent.effective_max_tokens(), 100);

    let mut messages = Vec::new();
    let mut session = AgentSession::new("stub".into(), "cli:test".into());
    let mut session_key = "cli:test".to_string();
    let store = FileSessionStore::new(tmp.path());
    let mut writer = tokio::io::sink();
    let initial_stats =
        crate::interface::cli::uds_session::compute_session_stats(&session_key, &messages);

    let cmd = AgentCommand::SetModel {
        id: Some("s".into()),
        model: Some("fireworks/qwen3p7-plus".into()),
        provider: None,
        model_id: None,
    };
    {
        let mut ctx = DispatchCtx {
            execution_state: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
            wire_mode: crate::interface::cli::uds_wire::ConnectionWireMode::legacy(),
            base_dir: tmp.path(),
            agent: &mut agent,
            messages: &mut messages,
            conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::interface::cli::uds_snapshots::ConversationSnapshotData::default(),
            )),
            state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                session.state_snapshot(0, None, 0, None),
            )),
            session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
            tool_catalogue_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session: &mut session,
            stdout: Some(&mut writer),
            session_key: &mut session_key,
            session_store: &store,
            ephemeral: false,
            system_prompt: "",
            cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
            turn_control: std::sync::Arc::default(),
            broadcast_tx: None,
            _ext_registry: None,
            client_tool_registry: new_client_tool_registry(),
            current_client_id: 0,
            subagent_registry: None,
            notification_rx: None,
            workflow_state: None,
            workflow_config: None,
            provider_reload: None,
            provider_reload_inputs: None,
            last_persisted_message_index: 0,
            durable_prefix_dirty: false,
        };
        assert!(!dispatch_command(cmd, &mut ctx).await);
    }
    assert_eq!(
        agent.effective_max_tokens(),
        100,
        "set_model limits must come from the active runtime, not a fresh registry reread"
    );
}

fn clamped_descriptor(
    provider: &str,
    model: &str,
    max_tokens: u32,
) -> crate::domain::catalogue::ModelDescriptor {
    use crate::domain::catalogue::*;
    ModelDescriptor {
        reference: ModelRef::parse(provider, model).unwrap(),
        display_name: None,
        transport: TransportKind::OpenAiCompletions,
        auth: AuthIdentity::ApiKey,
        base_url: None,
        auth_header: true,
        allow_remote_http: false,
        configured: true,
        capabilities: ModelCapabilities {
            input: Vec::new(),
            context_window: 0,
            max_tokens,
            context_window_explicit: false,
            max_tokens_explicit: true,
            reasoning: false,
            cost: ModelCost::default(),
        },
        availability: Availability::Runnable,
    }
}

#[tokio::test]
async fn dispatch_set_model_re_clamps_effective_max_tokens_from_the_published_catalogue() {
    // The agent is configured with max_tokens=100; switching to a model whose
    // published catalogue entry declares maxTokens=50 must clamp the effective
    // cap to 50. Deleting the descriptor-driven cap wiring in handle_set_model
    // makes this FAIL (the effective cap stays 100).
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"api":"openai-completions","baseUrl":"https://e.example/v1","apiKey":"k","models":[{"id":"qwen3p7-plus","maxTokens":50}]}}}"#,
    )
    .unwrap();

    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
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
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    });
    // Publish the model through the same runtime generation the session reads.
    agent
        .catalogue_store
        .publish(crate::domain::catalogue::CatalogueSnapshot::new(
            1,
            vec![clamped_descriptor("fireworks", "qwen3p7-plus", 50)],
        ));
    // Before the switch the stub model has no catalogue cap → configured value.
    assert_eq!(agent.effective_max_tokens(), 100);

    let mut messages = Vec::new();
    let mut session = AgentSession::new("stub".into(), "cli:test".into());
    let mut session_key = "cli:test".to_string();
    let store = FileSessionStore::new(tmp.path());
    let mut writer = tokio::io::sink();
    let initial_stats =
        crate::interface::cli::uds_session::compute_session_stats(&session_key, &messages);

    let cmd = AgentCommand::SetModel {
        id: Some("s".into()),
        model: Some("fireworks/qwen3p7-plus".into()),
        provider: None,
        model_id: None,
    };
    {
        let mut ctx = DispatchCtx {
            execution_state: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
            wire_mode: crate::interface::cli::uds_wire::ConnectionWireMode::legacy(),
            base_dir: tmp.path(),
            agent: &mut agent,
            messages: &mut messages,
            conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                crate::interface::cli::uds_snapshots::ConversationSnapshotData::default(),
            )),
            state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                session.state_snapshot(0, None, 0, None),
            )),
            session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
            tool_catalogue_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session: &mut session,
            stdout: Some(&mut writer),
            session_key: &mut session_key,
            session_store: &store,
            ephemeral: false,
            system_prompt: "",
            cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
            turn_control: std::sync::Arc::default(),
            broadcast_tx: None,
            _ext_registry: None,
            client_tool_registry: new_client_tool_registry(),
            current_client_id: 0,
            subagent_registry: None,
            notification_rx: None,
            workflow_state: None,
            workflow_config: None,
            provider_reload: None,
            provider_reload_inputs: None,
            last_persisted_message_index: 0,
            durable_prefix_dirty: false,
        };
        assert!(!dispatch_command(cmd, &mut ctx).await);
    }
    assert_eq!(
        agent.effective_max_tokens(),
        50,
        "a UDS model switch must re-clamp the effective cap to the catalogue's maxTokens"
    );
}
