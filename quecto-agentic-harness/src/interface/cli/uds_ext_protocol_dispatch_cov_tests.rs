use super::cov_tests::{cov_agent_with_registry, tool_reg};
use super::*;

#[tokio::test]
async fn dispatch_register_tools_rejects_disabled_core_shadow() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sandbox = crate::infrastructure::security::sandbox::Sandbox::new(
        Some(tmp.path().to_path_buf()),
        true,
    );
    let mut registry = crate::infrastructure::extensions::native::build_official_tool_registry(
        tmp.path().to_path_buf(),
        sandbox,
        Default::default(),
        false,
    );
    assert!(registry.disable_tool("bash"));
    assert!(
        !registry
            .definitions()
            .iter()
            .any(|def| def.name.as_ref() == "bash"),
        "disabled core tool should be hidden from model-visible definitions"
    );
    assert!(
        registry
            .descriptors()
            .iter()
            .any(|descriptor| descriptor.name() == "bash"),
        "disabled core tool should remain in the descriptor catalogue"
    );

    let mut agent = cov_agent_with_registry(registry);
    let mut messages = Vec::new();
    let mut session =
        super::super::uds_session::AgentSession::new("stub".into(), "cli:test".into());
    let mut session_key = "cli:test".to_string();
    let store =
        crate::infrastructure::persistence::session_store::FileSessionStore::new(tmp.path());
    let mut writer = tokio::io::sink();
    let client_registry = new_client_tool_registry();
    let state = session.state_snapshot(0, None, 0, None);
    let initial_stats = super::super::uds_session::compute_session_stats(&session_key, &messages);
    let tools = [tool_reg("bash")];
    let mut ctx = super::super::uds::DispatchCtx {
        execution_state: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
        wire_mode: super::super::uds_wire::ConnectionWireMode::legacy(),
        base_dir: tmp.path(),
        agent: &mut agent,
        messages: &mut messages,
        conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
            super::super::uds_snapshots::ConversationSnapshotData::default(),
        )),
        state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(state)),
        session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
        extension_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        session: &mut session,
        stdout: Some(&mut writer),
        session_key: &mut session_key,
        session_store: &store,
        ephemeral: false,
        system_prompt: "",
        cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(
            super::super::uds_cancel::CancelSlot::Idle,
        )),
        turn_control: std::sync::Arc::default(),
        broadcast_tx: None,
        ext_registry: None,
        client_tool_registry: client_registry.clone(),
        current_client_id: 456,
        subagent_registry: None,
        notification_rx: None,
        workflow_state: None,
        workflow_config: None,
        provider_reload: None,
        provider_reload_inputs: None,
        last_persisted_message_index: 0,
        durable_prefix_dirty: false,
    };

    dispatch_register_tools(&mut ctx, Some("shadow-disabled"), &tools).await;

    assert!(ctx.agent.tool_registry_extension_names().is_empty());
    assert!(
        client_registry.lock().unwrap().get(&456).is_none(),
        "rejected shadow must not create client tool state"
    );
}

#[tokio::test]
async fn dispatch_register_tools_preflights_registry_rejection_before_client_state_mutation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    registry.remove("blocked_ext");
    let mut agent = cov_agent_with_registry(registry);
    let mut messages = Vec::new();
    let mut session =
        super::super::uds_session::AgentSession::new("stub".into(), "cli:test".into());
    let mut session_key = "cli:test".to_string();
    let store =
        crate::infrastructure::persistence::session_store::FileSessionStore::new(tmp.path());
    let mut writer = tokio::io::sink();
    let client_registry = new_client_tool_registry();
    let state = session.state_snapshot(0, None, 0, None);
    let initial_stats = super::super::uds_session::compute_session_stats(&session_key, &messages);
    let tools = [tool_reg("blocked_ext")];
    let mut ctx = super::super::uds::DispatchCtx {
        execution_state: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
        wire_mode: super::super::uds_wire::ConnectionWireMode::legacy(),
        base_dir: tmp.path(),
        agent: &mut agent,
        messages: &mut messages,
        conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(
            super::super::uds_snapshots::ConversationSnapshotData::default(),
        )),
        state_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(state)),
        session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
        extension_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        session: &mut session,
        stdout: Some(&mut writer),
        session_key: &mut session_key,
        session_store: &store,
        ephemeral: false,
        system_prompt: "",
        cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(
            super::super::uds_cancel::CancelSlot::Idle,
        )),
        turn_control: std::sync::Arc::default(),
        broadcast_tx: None,
        ext_registry: None,
        client_tool_registry: client_registry.clone(),
        current_client_id: 789,
        subagent_registry: None,
        notification_rx: None,
        workflow_state: None,
        workflow_config: None,
        provider_reload: None,
        provider_reload_inputs: None,
        last_persisted_message_index: 0,
        durable_prefix_dirty: false,
    };

    dispatch_register_tools(&mut ctx, Some("deny-reg"), &tools).await;

    assert!(ctx.agent.tool_registry_extension_names().is_empty());
    assert!(
        client_registry.lock().unwrap().get(&789).is_none(),
        "staged UDS ownership must be rolled back when registry insertion rejects"
    );
}
