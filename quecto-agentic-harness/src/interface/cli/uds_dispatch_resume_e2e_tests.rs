use crate::domain::message::Message;
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::tools::subagent_registry::new_registry;
use crate::interface::cli::protocol::AgentCommand;

use super::cov_tests::Fixture;

#[tokio::test]
async fn e2e_resume_picker_lists_persisted_default_tui_chat_session() {
    let mut fx = Fixture::new();
    let persisted_key = crate::domain::session::Session::build_key("cli", "default");
    fx.store
        .save(&Session {
            key: persisted_key.clone(),
            messages: vec![Message::user("persisted message that /resume must offer")],
            workflow_run: None,
            subagent_roster: Vec::new(),
        })
        .await
        .unwrap();

    let listed = fx.store.list(None).await.unwrap();
    assert!(
        listed.iter().any(|summary| summary.key == persisted_key),
        "a TUI-owned persisted default session must be offered by bare /resume; listed={listed:?}"
    );

    let mut ctx = fx.ctx();
    assert!(
        !super::dispatch_command(
            AgentCommand::ListSessions {
                id: Some("resume-list".into()),
            },
            &mut ctx,
        )
        .await
    );
    assert!(
        !super::handle_resume_session(
            &mut ctx,
            Some("resume-select"),
            "resume_session",
            persisted_key.clone(),
        )
        .await
    );
    assert_eq!(*ctx.session_key, persisted_key);
    assert_eq!(ctx.messages.len(), 1);
}

/// A session file written by a pre-#1937 harness: the roster rows carry the
/// children's socket paths and pids as recovery authority.
async fn write_legacy_session_file(
    fx: &Fixture,
    key: &str,
    rows: serde_json::Value,
    workflow_run: serde_json::Value,
) {
    let path = fx._tmp.path().join("sessions").join(format!(
        "{}.json",
        crate::infrastructure::persistence::filename::sanitize_session_key(key)
    ));
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    let snapshot = serde_json::json!({
        "type": "snapshot",
        "key": key,
        "messages": [
            {"role":"user","content":"persisted transcript survives roster restore"},
            {"role":"assistant","content":"persisted answer"},
            {"role":"assistant","content":"child worker-live reported: done", "tool_calls": []},
        ],
        "workflow_run": workflow_run,
        "subagent_roster": rows,
    });
    tokio::fs::write(&path, format!("{snapshot}\n"))
        .await
        .unwrap();
    // The store must actually see the legacy file where it looks.
    assert!(
        fx.store.load(key).await.unwrap().is_some(),
        "legacy session file not found at {}",
        path.display()
    );
}

fn legacy_row(
    id: &str,
    socket: &std::path::Path,
    liveness: &str,
    status: &str,
) -> serde_json::Value {
    serde_json::json!({
        "agentUuid": id,
        "displayName": format!("worker-{id}"),
        "sessionKey": id,
        "socketPath": socket,
        "pid": std::process::id(),
        "liveness": liveness,
        "restoreReason": "legacy_unspecified",
        "parentId": "parent",
        "readOnly": true,
        "status": status,
        "deliveredMessageOrdinal": 3,
        "pendingMessageReports": [{"receipt":"r1","response":"past child message","ordinal":4}]
    })
}

fn feature_workflow_engine() -> crate::domain::workflow::WorkflowEngine {
    use crate::domain::workflow::{
        WorkflowConfig, WorkflowEngine, WorkflowTemplate, WorkflowTemplateStep,
    };
    let step = |key: &str| WorkflowTemplateStep {
        key: key.into(),
        label: key.into(),
        phase: "design".into(),
        guidance: None,
    };
    WorkflowEngine::new(
        WorkflowConfig {
            templates: vec![WorkflowTemplate {
                id: "feature".into(),
                label: "Feature".into(),
                description: "desc".into(),
                when_to_use: None,
                steps: vec![step("plan"), step("build")],
                guards: vec![],
            }],
            ..WorkflowConfig::default()
        },
        false,
    )
    .unwrap()
}

/// A listener at a persisted socket path that must never be connected to.
fn silent_listener(path: &std::path::Path) -> std::os::unix::net::UnixListener {
    let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
    listener.set_nonblocking(true).unwrap();
    listener
}

fn assert_never_probed(listener: &std::os::unix::net::UnixListener, what: &str) {
    match listener.accept() {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        other => panic!("{what}: restore probed the persisted socket: {other:?}"),
    }
}

/// #1937 / #1534: resuming a session written by an earlier harness restores
/// the transcript, the workflow run and the past child messages, performs
/// zero socket probes and pid compares, and leaves the operational roster
/// empty of the old children — live, unreachable, dead and detached alike.
#[tokio::test]
async fn e2e_resume_restores_history_without_readopting_children() {
    let mut fx = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let live_socket = dir.path().join("live.sock");
    let detached_socket = dir.path().join("detached.sock");
    let live_listener = silent_listener(&live_socket);
    let detached_listener = silent_listener(&detached_socket);
    let key = "cli:roster-resume";
    write_legacy_session_file(
        &fx,
        key,
        serde_json::json!([
            legacy_row("live", &live_socket, "live", "running"),
            legacy_row("unreachable", &dir.path().join("gone.sock"), "live", "idle"),
            legacy_row("dead", &dir.path().join("dead.sock"), "dead", "exited"),
            legacy_row("detached", &detached_socket, "detached", "idle"),
            {"displayName": "malformed"},
        ]),
        serde_json::json!({"template_id": "feature", "done": [true, false], "active_issue": null}),
    )
    .await;

    let registry = new_registry();
    // Rows the current harness held before switching away: gone too.
    registry.lock().unwrap().insert(
        "previous".into(),
        crate::infrastructure::tools::subagent_registry::SubagentEntry::new(
            dir.path().join("previous.sock"),
            0,
        ),
    );
    let workflow_state: crate::interface::shared::WorkflowStateHandle =
        std::sync::Arc::new(std::sync::Mutex::new(feature_workflow_engine()));
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(registry.clone());
        ctx.workflow_state = Some(workflow_state.clone());
        assert!(
            !super::handle_resume_session(
                &mut ctx,
                Some("resume-roster"),
                "resume_session",
                "roster-resume".into(),
            )
            .await
        );
    }

    assert_eq!(fx.session_key, key);
    assert_eq!(fx.messages.len(), 3, "transcript incl. past child message");
    assert_eq!(
        fx.messages[0].content,
        "persisted transcript survives roster restore"
    );
    assert_eq!(fx.messages[2].content, "child worker-live reported: done");
    let run = workflow_state
        .lock()
        .unwrap()
        .persisted_run()
        .expect("workflow run restored");
    assert_eq!(run.template_id.as_deref(), Some("feature"));
    assert_eq!(run.done, vec![true, false]);

    assert_never_probed(&live_listener, "live row");
    assert_never_probed(&detached_listener, "detached row");
    assert!(
        registry.lock().unwrap().is_empty(),
        "no operational child row from persisted records: {:?}",
        registry.lock().unwrap().keys().collect::<Vec<_>>()
    );
    let roster =
        crate::interface::cli::protocol::build_compact_subagent_roster(&Some(registry), None)
            .unwrap();
    assert!(roster.subagents.is_empty());
}

/// #1937: `new_session` after such a session leaves no old child either,
/// and the socket a legacy row named is never probed on the way out.
#[tokio::test]
async fn e2e_new_session_creates_no_child_row_and_probes_nothing() {
    let mut fx = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let live_socket = dir.path().join("live.sock");
    let listener = silent_listener(&live_socket);
    let key = "cli:roster-new";
    write_legacy_session_file(
        &fx,
        key,
        serde_json::json!([legacy_row("live", &live_socket, "live", "running")]),
        serde_json::Value::Null,
    )
    .await;
    let registry = new_registry();
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(registry.clone());
        assert!(
            !super::handle_resume_session(
                &mut ctx,
                Some("resume"),
                "resume_session",
                "roster-new".into()
            )
            .await
        );
        assert!(!super::handle_new_session(&mut ctx, Some("new"), "new_session").await);
    }
    assert_ne!(fx.session_key, key);
    assert!(fx.messages.is_empty());
    assert!(registry.lock().unwrap().is_empty());
    assert_never_probed(&listener, "live row");
    // The old session's history was saved on the way out, without pid/socket.
    let saved = fx.store.load(key).await.unwrap().unwrap();
    assert_eq!(saved.messages.len(), 3);
    assert!(
        saved.subagent_roster.is_empty(),
        "no operational child existed to snapshot"
    );
}
