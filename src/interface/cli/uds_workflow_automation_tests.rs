use super::*;
use crate::domain::session::{Session, SessionStore};

fn persisted_feature_run(done: Vec<bool>) -> crate::domain::workflow::WorkflowRunPersisted {
    crate::domain::workflow::WorkflowRunPersisted {
        template_id: Some("feature".into()),
        done,
        active_issue: None,
    }
}

fn make_workflow() -> crate::interface::shared::WorkflowStateHandle {
    std::sync::Arc::new(std::sync::Mutex::new(
        crate::domain::workflow::WorkflowEngine::new(
            crate::domain::workflow::WorkflowConfig::default(),
            false,
        )
        .unwrap(),
    ))
}

fn make_dispatch_test_agent() -> crate::application::agent_loop::AgentLoopImpl {
    crate::application::agent_loop::AgentLoopImpl::new(
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
            context_collapse_after_turns: u32::MAX,
            max_context_tokens: 190_000,
            progress_callback: None,
            streaming: false,
            effort: None,
            system_prompt_provider: None,
            audit_log: None,
        },
    )
}

#[tokio::test]
async fn new_session_resets_workflow_run_state() {
    let mut agent = make_dispatch_test_agent();
    let mut messages = vec![crate::domain::message::Message::user("old")];
    let mut session = AgentSession::new("stub".into(), "cli:test".into());
    let mut session_key = "cli:test".to_string();
    let tmp = tempfile::TempDir::new().unwrap();
    let store =
        crate::infrastructure::persistence::session_store::FileSessionStore::new(tmp.path());
    let mut writer = tokio::io::sink();
    let workflow = make_workflow();
    workflow
        .lock()
        .unwrap()
        .select_template("feature", None)
        .unwrap();
    workflow.lock().unwrap().check(1).unwrap();
    assert!(workflow.lock().unwrap().persisted_run().is_some());
    let mut ctx = DispatchCtx {
        base_dir: tmp.path(),
        agent: &mut agent,
        messages: &mut messages,
        conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        session: &mut session,
        stdout: &mut writer,
        session_key: &mut session_key,
        session_store: &store,
        ephemeral: false,
        system_prompt: "",
        cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
        broadcast_tx: None,
        ext_registry: None,
        client_tool_registry: crate::interface::cli::uds_ext_protocol::new_client_tool_registry(),
        current_client_id: 0,
        subagent_registry: None,
        notification_rx: None,
        workflow_state: Some(workflow.clone()),
        workflow_config: Some(crate::domain::workflow::WorkflowConfig::default()),
        provider_reload: None,
        provider_reload_inputs: None,
    };

    super::uds_dispatch::handle_new_session(&mut ctx, Some("n"), "new_session").await;

    assert!(workflow.lock().unwrap().persisted_run().is_none());
}

#[tokio::test]
async fn resume_session_restores_target_workflow_run_state() {
    let mut agent = make_dispatch_test_agent();
    let mut messages = vec![crate::domain::message::Message::user("current")];
    let mut session = AgentSession::new("stub".into(), "cli:test".into());
    let mut session_key = "cli:test".to_string();
    let tmp = tempfile::TempDir::new().unwrap();
    let store =
        crate::infrastructure::persistence::session_store::FileSessionStore::new(tmp.path());
    let key = Session::build_key("cli", "saved");
    store
        .save(&Session {
            key: key.clone(),
            messages: vec![crate::domain::message::Message::user("restored")],
            workflow_run: Some(persisted_feature_run(vec![true, false, false])),
        })
        .await
        .unwrap();
    let mut writer = tokio::io::sink();
    let workflow = make_workflow();
    let mut ctx = DispatchCtx {
        base_dir: tmp.path(),
        agent: &mut agent,
        messages: &mut messages,
        conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        session: &mut session,
        stdout: &mut writer,
        session_key: &mut session_key,
        session_store: &store,
        ephemeral: false,
        system_prompt: "",
        cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
        broadcast_tx: None,
        ext_registry: None,
        client_tool_registry: crate::interface::cli::uds_ext_protocol::new_client_tool_registry(),
        current_client_id: 0,
        subagent_registry: None,
        notification_rx: None,
        workflow_state: Some(workflow.clone()),
        workflow_config: Some(crate::domain::workflow::WorkflowConfig::default()),
        provider_reload: None,
        provider_reload_inputs: None,
    };

    super::uds_dispatch::handle_resume_session(
        &mut ctx,
        Some("r"),
        "resume_session",
        "saved".into(),
    )
    .await;

    let persisted = workflow.lock().unwrap().persisted_run().unwrap();
    assert_eq!(persisted.template_id.as_deref(), Some("feature"));
    assert!(persisted.done[0]);
}

#[tokio::test]
async fn resume_session_clears_workflow_when_target_has_none() {
    let mut agent = make_dispatch_test_agent();
    let mut messages = vec![crate::domain::message::Message::user("current")];
    let mut session = AgentSession::new("stub".into(), "cli:test".into());
    let mut session_key = "cli:test".to_string();
    let tmp = tempfile::TempDir::new().unwrap();
    let store =
        crate::infrastructure::persistence::session_store::FileSessionStore::new(tmp.path());
    store
        .save(&Session {
            key: Session::build_key("cli", "plain"),
            messages: vec![crate::domain::message::Message::user("plain")],
            workflow_run: None,
        })
        .await
        .unwrap();
    let mut writer = tokio::io::sink();
    let workflow = make_workflow();
    workflow
        .lock()
        .unwrap()
        .select_template("feature", None)
        .unwrap();
    workflow.lock().unwrap().check(1).unwrap();
    let mut ctx = DispatchCtx {
        base_dir: tmp.path(),
        agent: &mut agent,
        messages: &mut messages,
        conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        session: &mut session,
        stdout: &mut writer,
        session_key: &mut session_key,
        session_store: &store,
        ephemeral: false,
        system_prompt: "",
        cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
        broadcast_tx: None,
        ext_registry: None,
        client_tool_registry: crate::interface::cli::uds_ext_protocol::new_client_tool_registry(),
        current_client_id: 0,
        subagent_registry: None,
        notification_rx: None,
        workflow_state: Some(workflow.clone()),
        workflow_config: Some(crate::domain::workflow::WorkflowConfig::default()),
        provider_reload: None,
        provider_reload_inputs: None,
    };

    super::uds_dispatch::handle_resume_session(
        &mut ctx,
        Some("r"),
        "resume_session",
        "plain".into(),
    )
    .await;

    assert!(workflow.lock().unwrap().persisted_run().is_none());
}

#[tokio::test]
async fn set_workflow_automation_updates_config_and_engine() {
    let mut agent = make_dispatch_test_agent();
    let mut messages = Vec::new();
    let mut session = AgentSession::new("stub".into(), "cli:test".into());
    let mut session_key = "cli:test".to_string();
    let tmp = tempfile::TempDir::new().unwrap();
    let store =
        crate::infrastructure::persistence::session_store::FileSessionStore::new(tmp.path());
    let mut writer = tokio::io::sink();
    let workflow = std::sync::Arc::new(std::sync::Mutex::new(
        crate::domain::workflow::WorkflowEngine::new(
            crate::domain::workflow::WorkflowConfig::default(),
            false,
        )
        .unwrap(),
    ));
    let mut ctx = DispatchCtx {
        base_dir: tmp.path(),
        agent: &mut agent,
        messages: &mut messages,
        conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        session: &mut session,
        stdout: &mut writer,
        session_key: &mut session_key,
        session_store: &store,
        ephemeral: false,
        system_prompt: "",
        cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
        broadcast_tx: None,
        ext_registry: None,
        client_tool_registry: crate::interface::cli::uds_ext_protocol::new_client_tool_registry(),
        current_client_id: 0,
        subagent_registry: None,
        notification_rx: None,
        workflow_state: Some(workflow.clone()),
        workflow_config: Some(crate::domain::workflow::WorkflowConfig::default()),
        provider_reload: None,
        provider_reload_inputs: None,
    };

    super::uds_dispatch::handle_set_workflow_automation(
        &mut ctx,
        Some("wf"),
        "set_workflow_automation",
        Some(false),
        Some(false),
    )
    .await;

    let config = ctx.workflow_config.clone().unwrap();
    assert!(!config.auto_continue);
    assert!(!config.completion_nudge);
    let engine = workflow.lock().unwrap();
    assert!(!engine.auto_continue_enabled());
    assert!(!engine.completion_nudge_enabled());
}

#[test]
fn workflow_nudge_message_waits_for_selected_template() {
    let mut agent = make_dispatch_test_agent();
    let mut messages = Vec::new();
    let mut session = AgentSession::new("stub".into(), "cli:test".into());
    let mut session_key = "cli:test".to_string();
    let tmp = tempfile::TempDir::new().unwrap();
    let store =
        crate::infrastructure::persistence::session_store::FileSessionStore::new(tmp.path());
    let mut writer = tokio::io::sink();
    let workflow = std::sync::Arc::new(std::sync::Mutex::new(
        crate::domain::workflow::WorkflowEngine::new(
            crate::domain::workflow::WorkflowConfig::default(),
            false,
        )
        .unwrap(),
    ));
    let ctx = DispatchCtx {
        base_dir: tmp.path(),
        agent: &mut agent,
        messages: &mut messages,
        conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        session: &mut session,
        stdout: &mut writer,
        session_key: &mut session_key,
        session_store: &store,
        ephemeral: false,
        system_prompt: "",
        cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
        broadcast_tx: None,
        ext_registry: None,
        client_tool_registry: crate::interface::cli::uds_ext_protocol::new_client_tool_registry(),
        current_client_id: 0,
        subagent_registry: None,
        notification_rx: None,
        workflow_state: Some(workflow.clone()),
        workflow_config: Some(crate::domain::workflow::WorkflowConfig::default()),
        provider_reload: None,
        provider_reload_inputs: None,
    };

    assert!(super::workflow_nudge_message(&ctx).is_none());
    workflow
        .lock()
        .unwrap()
        .select_template("feature", None)
        .unwrap();
    assert!(
        super::workflow_nudge_message(&ctx)
            .unwrap()
            .contains("Workflow incomplete")
    );
}

#[test]
fn workflow_progress_fingerprint_changes_with_step_progress() {
    let mut agent = make_dispatch_test_agent();
    let mut messages = Vec::new();
    let mut session = AgentSession::new("stub".into(), "cli:test".into());
    let mut session_key = "cli:test".to_string();
    let tmp = tempfile::TempDir::new().unwrap();
    let store =
        crate::infrastructure::persistence::session_store::FileSessionStore::new(tmp.path());
    let mut writer = tokio::io::sink();
    let workflow = std::sync::Arc::new(std::sync::Mutex::new(
        crate::domain::workflow::WorkflowEngine::new(
            crate::domain::workflow::WorkflowConfig::default(),
            false,
        )
        .unwrap(),
    ));
    workflow
        .lock()
        .unwrap()
        .select_template("feature", None)
        .unwrap();
    let ctx = DispatchCtx {
        base_dir: tmp.path(),
        agent: &mut agent,
        messages: &mut messages,
        conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        session: &mut session,
        stdout: &mut writer,
        session_key: &mut session_key,
        session_store: &store,
        ephemeral: false,
        system_prompt: "",
        cancel_handle: std::sync::Arc::new(std::sync::Mutex::new(CancelSlot::Idle)),
        broadcast_tx: None,
        ext_registry: None,
        client_tool_registry: crate::interface::cli::uds_ext_protocol::new_client_tool_registry(),
        current_client_id: 0,
        subagent_registry: None,
        notification_rx: None,
        workflow_state: Some(workflow.clone()),
        workflow_config: Some(crate::domain::workflow::WorkflowConfig::default()),
        provider_reload: None,
        provider_reload_inputs: None,
    };
    let before = super::workflow_progress_fingerprint(&ctx).unwrap();
    workflow.lock().unwrap().check(1).unwrap();
    let after = super::workflow_progress_fingerprint(&ctx).unwrap();
    assert_ne!(before, after);
}
