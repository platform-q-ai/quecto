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

// ── Session switch releases the departing session's children (#1937 interim) ─

use crate::infrastructure::tools::subagent_monitor::spawn_monitor_task_unbound;
use crate::infrastructure::tools::subagent_registry::SubagentEntry;

const RELEASE_BOUND: std::time::Duration = std::time::Duration::from_secs(5);

/// A fake child: a listener the production monitor task connects to, exactly
/// as it connects to a launched child's socket. The accepted stream is the
/// child's view of its bound parent connection; its EOF is parent loss.
struct BoundChild {
    accepted: tokio::net::UnixStream,
    entry: SubagentEntry,
}

async fn bound_child(
    dir: &std::path::Path,
    name: &str,
    registry: &crate::infrastructure::tools::subagent_registry::SubagentRegistry,
) -> BoundChild {
    let socket_path = dir.join(format!("{name}.sock"));
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let mut entry = SubagentEntry::new(socket_path.clone(), 0);
    entry.display_name = name.to_string();
    let monitor = spawn_monitor_task_unbound(
        entry.agent_uuid.as_str().to_string(),
        socket_path,
        registry.clone(),
        None,
        None,
        None,
    );
    entry.monitor_handle = Some(std::sync::Arc::new(monitor));
    let (accepted, _) = tokio::time::timeout(RELEASE_BOUND, listener.accept())
        .await
        .expect("the monitor connects within the bound")
        .unwrap();
    BoundChild { accepted, entry }
}

/// The child's side of the bound connection reaches EOF within the bound:
/// the parent side was closed, which is what a launch-bound child reacts to.
async fn assert_parent_lost(mut accepted: tokio::net::UnixStream, what: &str) {
    use tokio::io::AsyncReadExt;
    let mut sink = [0u8; 1024];
    let eof = tokio::time::timeout(RELEASE_BOUND, async {
        loop {
            match accepted.read(&mut sink).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {} // the monitor's framed hello
            }
        }
    })
    .await;
    assert!(
        eof.is_ok(),
        "{what}: the bound parent connection stayed open"
    );
}

/// #1937 review: `new_session` must not strand the current session's live
/// launched child. Its monitor task — the owner of the bound parent
/// connection — is aborted before the row is cleared, so the child observes
/// parent loss and ends itself; no signal is sent and nothing is awaited.
#[tokio::test]
async fn e2e_new_session_releases_the_departing_childs_bound_connection() {
    let mut fx = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let registry = new_registry();
    let child = bound_child(dir.path(), "departing", &registry).await;
    registry.lock().unwrap().insert(
        child.entry.agent_uuid.as_str().to_string(),
        child.entry.clone(),
    );
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(registry.clone());
        assert!(!super::handle_new_session(&mut ctx, Some("new"), "new_session").await);
    }
    assert!(registry.lock().unwrap().is_empty());
    assert_parent_lost(child.accepted, "new_session").await;
    assert!(
        child.entry.monitor_handle.unwrap().is_finished(),
        "the monitor task was aborted, not merely dropped"
    );
}

/// #1937 review: resuming away from a session with a live launched child
/// releases that child the same way and restores the target session with an
/// empty roster; the target's legacy rows are still never probed.
#[tokio::test]
async fn e2e_resume_away_releases_the_departing_childs_bound_connection() {
    let mut fx = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let legacy_socket = dir.path().join("legacy.sock");
    let legacy_listener = silent_listener(&legacy_socket);
    write_legacy_session_file(
        &fx,
        "cli:elsewhere",
        serde_json::json!([legacy_row("legacy", &legacy_socket, "live", "running")]),
        serde_json::Value::Null,
    )
    .await;
    let registry = new_registry();
    let child = bound_child(dir.path(), "departing", &registry).await;
    registry.lock().unwrap().insert(
        child.entry.agent_uuid.as_str().to_string(),
        child.entry.clone(),
    );
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(registry.clone());
        assert!(
            !super::handle_resume_session(
                &mut ctx,
                Some("away"),
                "resume_session",
                "elsewhere".into()
            )
            .await
        );
    }
    assert_eq!(fx.session_key, "cli:elsewhere");
    assert!(registry.lock().unwrap().is_empty());
    assert_parent_lost(child.accepted, "resume_session").await;
    assert_never_probed(&legacy_listener, "legacy row of the resumed session");
}

/// A proxy-transport child's bridge accept loop and socket go with the row:
/// nothing can connect to the released child's bridge afterwards.
#[tokio::test]
async fn e2e_session_switch_tears_down_a_departing_childs_proxy_bridge() {
    let mut fx = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let bridge_socket = dir.path().join("bridge.sock");
    let listener = tokio::net::UnixListener::bind(&bridge_socket).unwrap();
    let accept_loop = tokio::spawn(async move {
        loop {
            let _ = listener.accept().await;
        }
    });
    let mut member = SubagentEntry::new(dir.path().join("member.sock"), 0);
    member.proxy_bridge_handle = Some(std::sync::Arc::new(accept_loop));
    member.proxy_bridge_socket = Some(bridge_socket.clone());
    let registry = new_registry();
    registry
        .lock()
        .unwrap()
        .insert("member".into(), member.clone());
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(registry.clone());
        assert!(!super::handle_new_session(&mut ctx, Some("new"), "new_session").await);
        // A second switch finds nothing left to release.
        assert!(!super::handle_new_session(&mut ctx, Some("new-2"), "new_session").await);
    }
    assert!(registry.lock().unwrap().is_empty());
    assert!(!bridge_socket.exists(), "the bridge socket is removed");
    let handle = member.proxy_bridge_handle.unwrap();
    tokio::time::timeout(RELEASE_BOUND, async {
        while !handle.is_finished() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the accept loop is aborted");
}
