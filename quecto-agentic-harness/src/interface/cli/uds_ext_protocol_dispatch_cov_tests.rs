use super::cov_tests::{cov_agent_with_registry, tool_reg};
use super::*;
use crate::domain::tool_descriptor::ProfileAvailabilityScope;

#[tokio::test]
async fn dispatch_register_tools_rejects_disabled_core_shadow() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sandbox =
        crate::infrastructure::security::sandbox::Sandbox::new(Some(tmp.path().to_path_buf()));
    let mut registry = crate::infrastructure::extensions::native::build_official_tool_registry(
        tmp.path().to_path_buf(),
        sandbox,
        Default::default(),
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
        tool_catalogue_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
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
        _ext_registry: None,
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

    assert!(ctx.agent.runtime_tool_names().is_empty());
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
        tool_catalogue_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
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
        _ext_registry: None,
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

    assert!(ctx.agent.runtime_tool_names().is_empty());
    assert!(
        client_registry.lock().unwrap().get(&789).is_none(),
        "staged UDS ownership must be rolled back when registry insertion rejects"
    );
}

#[tokio::test]
async fn dispatch_register_tools_accepts_stable_id_for_policy_mutation() {
    let tmp = tempfile::TempDir::new().unwrap();
    let registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
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
    let mut tool = tool_reg("weather");
    tool.stable_id = Some("com.example.weather.v1".into());
    let tools = [tool];
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
        tool_catalogue_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
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
        _ext_registry: None,
        client_tool_registry: client_registry,
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

    dispatch_register_tools(&mut ctx, Some("reg-stable"), &tools).await;
    let entry = ctx
        .agent
        .tool_catalogue_entries()
        .into_iter()
        .find(|entry| entry.name == "weather")
        .expect("registered UDS tool");
    assert_eq!(entry.stable_id.as_ref(), "com.example.weather.v1");

    let reconciliation = ctx
        .agent
        .request_tool_policy_mutation(
            &[crate::domain::tool::ToolPolicyMutation::set_scope(
                "com.example.weather.v1",
                ProfileAvailabilityScope::Child,
                "stable id mutation",
            )],
            crate::domain::tool::ToolPolicyApplyMode::ImmediateIfIdle,
        )
        .expect("policy applies immediately while idle");
    assert_eq!(
        reconciliation.results[0].status,
        crate::domain::tool::ToolPolicyMutationStatus::Applied
    );
    let entry = ctx
        .agent
        .tool_catalogue_entries()
        .into_iter()
        .find(|entry| entry.name == "weather")
        .expect("registered UDS tool");
    assert_eq!(entry.profile_scope, Some(ProfileAvailabilityScope::Child));
}
