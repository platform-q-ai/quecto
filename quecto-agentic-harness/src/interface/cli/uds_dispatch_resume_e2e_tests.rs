use crate::application::sessions::ports::SessionStore;
use crate::domain::message::Message;
use crate::domain::session::Session;
use crate::infrastructure::tools::subagent_registry::new_registry;
use crate::interface::cli::protocol::AgentCommand;

use super::fixture_tests::Fixture;

#[tokio::test]
async fn e2e_resume_picker_lists_persisted_default_tui_chat_session() {
    let mut fx = Fixture::new();
    let persisted_key = crate::domain::session::Session::build_key("cli", "default");
    crate::interface::cli::uds::dispatch_session_roster_tests::seed_home(&fx.store, &persisted_key)
        .await;
    fx.store
        .save(&Session {
            key: crate::domain::session_identity::SessionIdentity::from_persisted_key(
                persisted_key.clone(),
            ),
            messages: vec![Message::user("persisted message that /resume must offer")],
            workflow_run: None,
            subagent_roster: Vec::new(),
        })
        .await
        .unwrap();

    let listed = fx
        .store
        .list(&crate::application::sessions::dto::SessionListQuery::All)
        .await
        .unwrap();
    assert!(
        listed.iter().any(|summary| summary.key == persisted_key),
        "a TUI-owned persisted default session must be offered by bare /resume; listed={listed:?}"
    );

    let mut ctx = fx.ctx();
    assert!(
        !super::dispatch_command(
            AgentCommand::ListSessions {
                scope: Default::default(),
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
    assert_eq!(ctx.sessions.current_session_key().await, persisted_key);
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
    crate::interface::cli::uds::dispatch_session_roster_tests::seed_home(&fx.store, key).await;
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
        fx.store
            .load(&crate::domain::session_identity::SessionIdentity::from_persisted_key(key))
            .await
            .unwrap()
            .is_some(),
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
    fx.set_subagent_registry(registry.clone());
    {
        let mut ctx = fx.ctx();
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

    assert_eq!(fx.current_session_key(), key);
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
    fx.set_subagent_registry(registry.clone());
    {
        let mut ctx = fx.ctx();
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
    assert_ne!(fx.current_session_key(), key);
    assert!(fx.messages.is_empty());
    assert!(registry.lock().unwrap().is_empty());
    assert_never_probed(&listener, "live row");
    // The old session's history was saved on the way out, without pid/socket.
    let saved = fx
        .store
        .load(&crate::domain::session_identity::SessionIdentity::from_persisted_key(key))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.messages.len(), 3);
    assert!(
        saved.subagent_roster.is_empty(),
        "no operational child existed to snapshot"
    );
}

// ── Session switches settle the departing session's children (#1938) ─────────

use crate::infrastructure::tools::subagent_monitor::spawn_monitor_task;
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, SubagentRegistry};
use std::sync::{Arc, Mutex};

const SWITCH_BOUND: std::time::Duration = std::time::Duration::from_secs(20);

/// A fake launched child: a listener the production monitor task connects
/// to, exactly as it connects to a launched child's socket, and that
/// acknowledges a `shutdown` sent over the same socket by closing every
/// connection it holds — the child's graceful exit as its parent sees it
/// (monitor EOF). Records every request.
struct AckingChild {
    entry: SubagentEntry,
    requests: Arc<Mutex<Vec<serde_json::Value>>>,
    monitor: Arc<tokio::task::JoinHandle<()>>,
}

async fn acking_child(
    dir: &std::path::Path,
    name: &str,
    registry: &SubagentRegistry,
) -> AckingChild {
    let socket_path = dir.join(format!("{name}.sock"));
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let requests: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
    type Held = (
        tokio::io::BufReader<tokio::io::ReadHalf<tokio::net::UnixStream>>,
        tokio::io::WriteHalf<tokio::net::UnixStream>,
    );
    let held: Arc<Mutex<Vec<Held>>> = Arc::new(Mutex::new(Vec::new()));
    let accepted = Arc::new(tokio::sync::Notify::new());
    let (seen, streams, first) = (requests.clone(), held.clone(), accepted.clone());
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            first.notify_one();
            let (read, mut write) = tokio::io::split(stream);
            let mut reader = tokio::io::BufReader::new(read);
            // A monitor connection opens with an empty hello frame and is
            // held; a command connection carries one JSON request.
            let Ok(Some(incoming)) =
                quecto_line_io::read_frame_or_legacy_line(&mut reader, 64 * 1024).await
            else {
                continue;
            };
            let bytes = match incoming {
                quecto_line_io::Incoming::Frame(b) | quecto_line_io::Incoming::LegacyLine(b) => b,
            };
            let Ok(request) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                streams.lock().unwrap().push((reader, write));
                continue;
            };
            seen.lock().unwrap().push(request.clone());
            if request["type"] == "shutdown" {
                use tokio::io::AsyncWriteExt;
                let reply = serde_json::json!({
                    "type": "response", "id": request["id"], "command": "shutdown", "success": true,
                });
                let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
                let _ = write.shutdown().await;
                // The child exits: every connection it held closes.
                streams.lock().unwrap().clear();
                return;
            }
        }
    });
    let mut entry = SubagentEntry::new(socket_path.clone(), 0);
    entry.display_name = name.to_string();
    entry.launch_generation =
        Some(crate::infrastructure::processes::parent_control::next_launch_generation());
    let observer = crate::composition::subagent_lifecycle::build_lifecycle_use_cases(
        registry.clone(),
        None,
        None,
    )
    .observe_exit;
    let monitor = Arc::new(spawn_monitor_task(
        crate::infrastructure::tools::subagent_monitor::MonitorSpec {
            agent_id: entry.agent_uuid.as_str().to_string(),
            socket_path,
            registry: registry.clone(),
            notify_tx: None,
            broadcast_tx: None,
            parent_id: None,
            observer,
            parent_control: None,
        },
    ));
    entry.monitor_handle = Some(monitor.clone());
    tokio::time::timeout(SWITCH_BOUND, accepted.notified())
        .await
        .expect("the monitor connects within the bound");
    AckingChild {
        entry,
        requests,
        monitor,
    }
}

fn production_fleet(
    registry: &SubagentRegistry,
) -> Arc<crate::application::subagents::use_cases::TerminateAllDelegatedAgents> {
    crate::composition::subagent_teardown::build_fleet_teardown(
        crate::composition::subagent_teardown::FleetTeardownWiring {
            owner: crate::domain::ids::AgentUuid::new("root"),
            registry: registry.clone(),
            broadcast_tx: None,
            notify_tx: None,
            harness_lifecycle:
                crate::infrastructure::tools::harness_lifecycle::new_shared_harness_lifecycle(),
        },
    )
}

/// #1938: `new_session` settles the current session's live launched child
/// through the fleet teardown before the roster is replaced: the child is
/// asked to shut down over its edge, its exit is observed through its
/// monitor, its row is compensated and pruned, and the harness continues.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_new_session_settles_the_departing_child_before_the_roster_is_replaced() {
    let mut fx = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let registry = new_registry();
    let child = acking_child(dir.path(), "departing", &registry).await;
    registry.lock().unwrap().insert(
        child.entry.agent_uuid.as_str().to_string(),
        child.entry.clone(),
    );
    let fleet = production_fleet(&registry);
    fx.set_subagent_registry(registry.clone());
    {
        let mut ctx = fx.ctx();
        ctx.fleet_teardown = Some(fleet.clone());
        let switched = tokio::time::timeout(
            SWITCH_BOUND,
            super::handle_new_session(&mut ctx, Some("new"), "new_session"),
        )
        .await
        .expect("bounded");
        assert!(!switched);
        // A second switch finds nothing left to settle.
        assert!(!super::handle_new_session(&mut ctx, Some("new-2"), "new_session").await);
    }
    assert_ne!(fx.current_session_key(), "cli:test");
    let requests = child.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0]["type"], "shutdown");
    assert_eq!(requests[0]["reason"], "operator_request");
    assert!(
        registry.lock().unwrap().is_empty(),
        "settled and pruned before the roster was replaced"
    );
    tokio::time::timeout(SWITCH_BOUND, async {
        while !child.monitor.is_finished() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the compensation ends the monitor task");
}

/// #1938: resuming away from a session with a live launched child settles
/// that child the same way and restores the target session with an empty
/// roster; the target's legacy rows are still never probed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn e2e_resume_away_settles_the_departing_child_and_probes_no_legacy_socket() {
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
    let child = acking_child(dir.path(), "departing", &registry).await;
    registry.lock().unwrap().insert(
        child.entry.agent_uuid.as_str().to_string(),
        child.entry.clone(),
    );
    let fleet = production_fleet(&registry);
    fx.set_subagent_registry(registry.clone());
    {
        let mut ctx = fx.ctx();
        ctx.fleet_teardown = Some(fleet);
        let switched = tokio::time::timeout(
            SWITCH_BOUND,
            super::handle_resume_session(
                &mut ctx,
                Some("away"),
                "resume_session",
                "elsewhere".into(),
            ),
        )
        .await
        .expect("bounded");
        assert!(!switched);
    }
    assert_eq!(fx.current_session_key(), "cli:elsewhere");
    assert_eq!(child.requests.lock().unwrap().len(), 1);
    assert!(registry.lock().unwrap().is_empty());
    assert_never_probed(&legacy_listener, "legacy row of the resumed session");
}

/// #1938: a departing child that cannot be settled refuses the transition
/// explicitly — the current session, its key and its roster are kept — so
/// ownership of a live child is never silently dropped.
#[tokio::test]
async fn e2e_a_session_switch_is_refused_while_a_departing_child_cannot_be_settled() {
    use crate::application::subagents::use_cases::teardown_fakes::*;
    let mut fx = Fixture::new();
    let lifecycle = FakeLifecycle::new(root_tree());
    let routing = FakeRouting::new();
    let fleet = fake_fleet(lifecycle, routing, FakeSpawner::new());
    fleet.termination.conclude_child_with(
        "A",
        crate::application::subagents::ports::TerminationConclusion::StillRunning(
            "ignores TERM and KILL".into(),
        ),
    );
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "A".into(),
        SubagentEntry::new(std::path::PathBuf::from("/tmp/a.sock"), 0),
    );
    fx.messages.push(Message::user("kept"));
    fx.set_subagent_registry(registry.clone());
    {
        let mut ctx = fx.ctx();
        ctx.fleet_teardown = Some(fleet.fleet.clone());
        assert!(!super::handle_new_session(&mut ctx, Some("new"), "new_session").await);
        assert!(
            !super::handle_resume_session(&mut ctx, Some("away"), "resume_session", "x".into())
                .await
        );
    }
    assert_eq!(
        fx.current_session_key(),
        "cli:test",
        "the current session is kept"
    );
    assert_eq!(fx.messages.len(), 1, "nothing was cleared");
    assert_eq!(
        registry.lock().unwrap().len(),
        1,
        "the roster was not touched"
    );
    // The unsettled child keeps its row with the claim lifted: D settled.
    assert_eq!(
        fleet.registry.phase("A"),
        crate::application::subagents::use_cases::lifecycle_fakes::Phase::Live
    );
}

/// A loop without a fleet teardown (no teardown graph) refuses to switch
/// away from live delegated rows rather than dropping them, and replaces a
/// roster of records only.
#[tokio::test]
async fn e2e_without_a_fleet_teardown_live_delegated_rows_refuse_the_switch() {
    let mut fx = Fixture::new();
    let registry = new_registry();
    let mut live = SubagentEntry::new(std::path::PathBuf::from("/tmp/live.sock"), 0);
    live.launch_generation =
        Some(crate::infrastructure::processes::parent_control::next_launch_generation());
    registry.lock().unwrap().insert("live".into(), live);
    fx.set_subagent_registry(registry.clone());
    {
        let mut ctx = fx.ctx();
        assert!(!super::handle_new_session(&mut ctx, Some("new"), "new_session").await);
    }
    assert_eq!(fx.current_session_key(), "cli:test");
    assert_eq!(registry.lock().unwrap().len(), 1);
    // A record-only row (no launch generation) is replaced.
    registry.lock().unwrap().clear();
    registry.lock().unwrap().insert(
        "record".into(),
        SubagentEntry::new(std::path::PathBuf::from("/tmp/record.sock"), 0),
    );
    fx.set_subagent_registry(registry.clone());
    {
        let mut ctx = fx.ctx();
        assert!(!super::handle_new_session(&mut ctx, Some("new"), "new_session").await);
    }
    assert_ne!(fx.current_session_key(), "cli:test");
    assert!(registry.lock().unwrap().is_empty());
}
