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
async fn dispatch_set_model_re_clamps_effective_max_tokens() {
    // The agent is configured with max_tokens=100; after switching to a model
    // whose registry maxTokens is 50, the effective cap must clamp to 50.
    // Deleting the registry-lookup + set_model cap wiring in handle_set_model
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
        system_prompt_provider: None,
        audit_log: None,
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
            base_dir: tmp.path(),
            agent: &mut agent,
            messages: &mut messages,
            conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
                session.state_snapshot(0, None, 0),
            )),
            session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
            extension_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
            busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            session: &mut session,
            stdout: &mut writer,
            session_key: &mut session_key,
            session_store: &store,
            ephemeral: false,
            system_prompt: "",
            cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
            turn_control: std::sync::Arc::default(),
            broadcast_tx: None,
            ext_registry: None,
            client_tool_registry: new_client_tool_registry(),
            current_client_id: 0,
            subagent_registry: None,
            notification_rx: None,
            workflow_state: None,
            workflow_config: None,
            provider_reload: None,
            provider_reload_inputs: None,
            last_persisted_message_index: 0,
        };
        assert!(!dispatch_command(cmd, &mut ctx).await);
    }
    assert_eq!(
        agent.effective_max_tokens(),
        50,
        "a UDS model switch must re-clamp the effective cap to the new model's registry maxTokens"
    );
}
