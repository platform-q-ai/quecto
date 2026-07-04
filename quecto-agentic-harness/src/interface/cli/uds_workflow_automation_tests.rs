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
            context_collapse_after_tool_calls: u32::MAX,
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
    let initial_stats =
        crate::interface::cli::uds_session::compute_session_stats(&session_key, &messages);
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
        client_tool_registry: crate::interface::cli::uds_ext_protocol::new_client_tool_registry(),
        current_client_id: 0,
        subagent_registry: None,
        notification_rx: None,
        workflow_state: Some(workflow.clone()),
        workflow_config: Some(crate::domain::workflow::WorkflowConfig::default()),
        provider_reload: None,
        provider_reload_inputs: None,
        last_persisted_message_index: 0,
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
    let initial_stats =
        crate::interface::cli::uds_session::compute_session_stats(&session_key, &messages);
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
        client_tool_registry: crate::interface::cli::uds_ext_protocol::new_client_tool_registry(),
        current_client_id: 0,
        subagent_registry: None,
        notification_rx: None,
        workflow_state: Some(workflow.clone()),
        workflow_config: Some(crate::domain::workflow::WorkflowConfig::default()),
        provider_reload: None,
        provider_reload_inputs: None,
        last_persisted_message_index: 0,
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
    let initial_stats =
        crate::interface::cli::uds_session::compute_session_stats(&session_key, &messages);
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
        client_tool_registry: crate::interface::cli::uds_ext_protocol::new_client_tool_registry(),
        current_client_id: 0,
        subagent_registry: None,
        notification_rx: None,
        workflow_state: Some(workflow.clone()),
        workflow_config: Some(crate::domain::workflow::WorkflowConfig::default()),
        provider_reload: None,
        provider_reload_inputs: None,
        last_persisted_message_index: 0,
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
    let initial_stats =
        crate::interface::cli::uds_session::compute_session_stats(&session_key, &messages);
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
        client_tool_registry: crate::interface::cli::uds_ext_protocol::new_client_tool_registry(),
        current_client_id: 0,
        subagent_registry: None,
        notification_rx: None,
        workflow_state: Some(workflow.clone()),
        workflow_config: Some(crate::domain::workflow::WorkflowConfig::default()),
        provider_reload: None,
        provider_reload_inputs: None,
        last_persisted_message_index: 0,
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
    let initial_stats =
        crate::interface::cli::uds_session::compute_session_stats(&session_key, &messages);
    let ctx = DispatchCtx {
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
        client_tool_registry: crate::interface::cli::uds_ext_protocol::new_client_tool_registry(),
        current_client_id: 0,
        subagent_registry: None,
        notification_rx: None,
        workflow_state: Some(workflow.clone()),
        workflow_config: Some(crate::domain::workflow::WorkflowConfig::default()),
        provider_reload: None,
        provider_reload_inputs: None,
        last_persisted_message_index: 0,
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

#[tokio::test]
async fn drain_refreshes_busy_state_snapshot_per_turn() {
    // #899: a busy workflow child inspected mid-workflow must see CURRENT state,
    // not the pre-turn/initial snapshot. The snapshots are refreshed after each
    // inner turn inside the drain loop — so message count and workflow advance
    // step-by-step instead of staying frozen until the whole command returns.
    let mut agent = make_dispatch_test_agent();
    let mut messages = Vec::new();
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
    // The shared state snapshot starts at the pre-turn (initial) view: no
    // workflow attached, zero messages — exactly what a busy child wrongly
    // served before #899.
    let initial_state = session.state_snapshot(0, None, 0);
    let state_snapshot = std::sync::Arc::new(tokio::sync::RwLock::new(initial_state));
    let initial_stats =
        crate::interface::cli::uds_session::compute_session_stats(&session_key, &messages);
    let mut ctx = DispatchCtx {
        base_dir: tmp.path(),
        agent: &mut agent,
        messages: &mut messages,
        conversation_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        state_snapshot: state_snapshot.clone(),
        session_stats_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(initial_stats)),
        extension_snapshot: std::sync::Arc::new(tokio::sync::RwLock::new(Vec::new())),
        busy: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        turn_control: std::sync::Arc::default(),
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
        last_persisted_message_index: 0,
    };

    // Two pending messages drive TWO inner turns through the drain loop, so the
    // refresh fires per turn across a multi-turn chain (AC2) — not just once when
    // the whole command returns. Each turn adds a user + assistant message, so a
    // step-by-step refresh leaves the snapshot at the full post-chain count.
    ctx.session.enqueue_pending("do the work".to_string());
    ctx.session.enqueue_pending("keep going".to_string());
    super::drain_and_run_pending(&mut ctx).await;

    let expected_count = ctx.messages.len();
    assert!(
        expected_count >= 4,
        "two inner turns should each add a user+assistant message, got {expected_count}"
    );

    // AC1: state snapshot tracks the live workflow + advanced message count.
    let snap = state_snapshot.read().await;
    assert_eq!(
        snap.message_count, expected_count,
        "busy state snapshot must reflect the post-chain message count, not the initial pre-turn view"
    );
    assert!(
        snap.workflow.is_some(),
        "busy state snapshot must reflect the selected workflow, not the initial pre-turn view"
    );
    drop(snap);

    // AC3: the conversation + session_stats snapshots refresh per turn too, not
    // just get_state — a regression dropping any of those refresh calls is caught.
    let convo = ctx.conversation_snapshot.read().await;
    assert_eq!(
        convo.len(),
        expected_count,
        "busy conversation snapshot must advance with the conversation, not stay empty"
    );
    drop(convo);
    let stats = ctx.session_stats_snapshot.read().await;
    assert_eq!(
        stats.total_messages, expected_count,
        "busy session_stats snapshot must reflect the post-chain message count"
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
    let initial_stats =
        crate::interface::cli::uds_session::compute_session_stats(&session_key, &messages);
    let ctx = DispatchCtx {
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
        client_tool_registry: crate::interface::cli::uds_ext_protocol::new_client_tool_registry(),
        current_client_id: 0,
        subagent_registry: None,
        notification_rx: None,
        workflow_state: Some(workflow.clone()),
        workflow_config: Some(crate::domain::workflow::WorkflowConfig::default()),
        provider_reload: None,
        provider_reload_inputs: None,
        last_persisted_message_index: 0,
    };
    let before = super::workflow_progress_fingerprint(&ctx).unwrap();
    workflow.lock().unwrap().check(1).unwrap();
    let after = super::workflow_progress_fingerprint(&ctx).unwrap();
    assert_ne!(before, after);
}
