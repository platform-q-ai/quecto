use crate::protocol::client::Client;
use crate::shell::app::App;
use crate::shell::connection::TabId;
use crate::shell::terminal::Terminal;
use crate::shell::workspace_manifest::{WorkspaceManifestStore, WorkspaceTabEntry};

fn app() -> App {
    let client = Client::disconnected_for_tests();
    let mut term = Terminal::new();
    term.set_size_for_tests(80, 24);
    App::new(term, client)
}

#[test]
fn resume_selector_lists_workspaces_above_sessions() {
    let mut a = app();
    let dir = tempfile::tempdir().unwrap();
    let mpath = dir.path().join("m.json");
    let mut store = WorkspaceManifestStore::new();
    store.upsert(App::test_workspace_manifest(
        "ws-demo",
        vec![WorkspaceTabEntry {
            tab_id: 0,
            session_key: Some("s0".into()),
            name: Some("main".into()),
            summary: None,
        }],
        0,
    ));
    store.store(&mpath).unwrap();

    let data = serde_json::json!({
        "sessions": [
            {"name": "alpha", "messageCount": 2}
        ]
    });
    a.open_resume_selector_at(&data, &mpath);
    let sel = a.ac().sessions.resume_selector.as_ref().expect("selector");
    assert!(sel.item_count() >= 2, "count={}", sel.item_count());
    let values: Vec<_> = sel
        .items_for_tests()
        .iter()
        .map(|i| i.value.clone())
        .collect();
    assert!(
        values[0].starts_with("workspace:"),
        "workspace first: {values:?}"
    );
    assert!(
        values.iter().any(|v| v.starts_with("session:")),
        "session present: {values:?}"
    );
}

#[test]
fn apply_workspace_manifest_opens_tabs() {
    let mut a = app();
    a.ac_mut().agent_connected = true;
    let manifest = App::test_workspace_manifest(
        "ws",
        vec![
            WorkspaceTabEntry {
                tab_id: 0,
                session_key: Some("live-sess".into()),
                name: Some("one".into()),
                summary: None,
            },
            WorkspaceTabEntry {
                tab_id: 1,
                session_key: Some("dead-sess".into()),
                name: Some("two".into()),
                summary: None,
            },
        ],
        1,
    );
    a.apply_workspace_manifest(&manifest);
    assert!(a.tabs.len() >= 2, "tabs={}", a.tabs.len());
    assert_eq!(a.active_tab, TabId(1));
    assert_eq!(a.conn_for(TabId(1)).unwrap().name.as_deref(), Some("two"));
}

#[test]
fn bare_session_selection_still_current_tab() {
    let mut a = app();
    // Live writer so resume commands are observable on the active tab sender.
    let (mut live, mut rx) = crate::shell::connection::Connection::live_for_tests();
    live.set_tab_for_tests(TabId::MASTER);
    a.ac_mut().transport = live;
    a.ac_mut().agent_connected = true;
    assert_eq!(a.active_tab, TabId::MASTER);
    a.apply_resume_selection("session:my-key");
    let line = rx
        .try_recv()
        .expect("session: prefix must send resume on active tab");
    assert!(line.contains("resume_session"), "wire={line}");
    assert!(line.contains("my-key"), "wire={line}");
    assert_eq!(a.active_tab, TabId::MASTER, "must not open/switch tabs");
    a.apply_resume_selection("plain-key");
    let line = rx
        .try_recv()
        .expect("bare key must send resume on active tab");
    assert!(line.contains("resume_session"), "wire={line}");
    assert!(line.contains("plain-key"), "wire={line}");
    assert_eq!(a.active_tab, TabId::MASTER);
    assert_eq!(a.tabs.len(), 1);
}

#[test]
fn unknown_workspace_notifies_without_changing_tabs() {
    let mut a = app();
    let n = a.tabs.len();
    let active = a.active_tab;
    a.restore_workspace_from_path(
        "nope",
        std::path::Path::new("/tmp/does-not-exist-1465.json"),
    );
    assert_eq!(a.tabs.len(), n);
    assert_eq!(a.active_tab, active);
    let msgs = a.notifications.messages().join("\n");
    assert!(
        msgs.contains("Unknown workspace"),
        "expected unknown-workspace notice, got {msgs:?}"
    );
}

#[test]
fn restore_workspace_counts_legacy_session_key_rows() {
    let mut a = app();
    a.ac_mut().agent_connected = true;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("workspace-manifests.json");
    std::fs::write(
        &path,
        r#"{"version":1,"workspaces":[{"workspace_id":"ws","label":"legacy","last_active_unix_s":0,"active_index":0,"tabs":[{"tab_id":0,"session":"legacy-main","name":"one"}],"updated_unix_s":1}]}"#,
    )
    .unwrap();

    a.restore_workspace_from_path("ws", &path);

    let msgs = a.notifications.messages().join("\n");
    assert!(
        msgs.contains("Workspace 'ws' restored (1 resumed, 0 deferred/failed)"),
        "legacy workspace tab session keys must be resumable, not report 0/0: {msgs:?}"
    );
}

#[test]
fn apply_workspace_manifest_queues_deferred_resume_for_disconnected_tabs() {
    let mut a = app();
    // MASTER stays disconnected; restore must queue deferred resume instead of
    // silently counting failure with no retry path.
    a.ac_mut().agent_connected = false;
    let manifest = App::test_workspace_manifest(
        "ws",
        vec![
            WorkspaceTabEntry {
                tab_id: 0,
                session_key: Some("live-sess".into()),
                name: Some("one".into()),
                summary: None,
            },
            WorkspaceTabEntry {
                tab_id: 1,
                session_key: Some("dead-sess".into()),
                name: Some("two".into()),
                summary: None,
            },
        ],
        0,
    );
    a.apply_workspace_manifest(&manifest);
    assert!(
        a.conn_for(TabId::MASTER)
            .unwrap()
            .pending_session_resume
            .as_deref()
            == Some("live-sess"),
        "AC6: disconnected tab must retain deferred session resume"
    );
    assert!(
        a.conn_for(TabId(1))
            .unwrap()
            .pending_session_resume
            .as_deref()
            == Some("dead-sess")
    );
}

#[test]
fn apply_workspace_manifest_updates_transport_tab_id_in_production_path() {
    let mut a = app();
    // Force allocate-then-remap path by requesting a non-sequential tab id.
    let manifest = App::test_workspace_manifest(
        "ws",
        vec![
            WorkspaceTabEntry {
                tab_id: 0,
                session_key: None,
                name: Some("one".into()),
                summary: None,
            },
            WorkspaceTabEntry {
                tab_id: 7,
                session_key: Some("s7".into()),
                name: Some("seven".into()),
                summary: None,
            },
        ],
        1,
    );
    a.apply_workspace_manifest(&manifest);
    let state = a.conn_for(TabId(7)).expect("tab 7 present");
    assert_eq!(
        state.transport.tab().0,
        7,
        "production remap must update Connection.tab, not only the HashMap key"
    );
}

#[test]
fn apply_workspace_manifest_reattaches_live_registry_socket_on_connected_tab() {
    use crate::shell::tab_registry::{TabAgentRecord, TabAgentRegistry, TabAgentStatus};
    use std::os::unix::net::UnixListener;

    let mut a = app();
    a.ac_mut().agent_connected = true;
    a.ac_mut().session_key = Some("cli:fresh-master".into());
    a.ac_mut().socket_path = Some(std::path::PathBuf::from("/tmp/fresh-master.sock"));
    let (fresh, mut fresh_rx) = crate::shell::connection::Connection::live_for_tests();
    a.ac_mut().transport = fresh;

    let dir = tempfile::tempdir().unwrap();
    let live_sock = dir.path().join("old-master.sock");
    let _listener = UnixListener::bind(&live_sock).unwrap();
    let rpath = dir.path().join("registry.json");
    let mut reg = TabAgentRegistry::new();
    reg.upsert(TabAgentRecord {
        tab_id: 0,
        pid: Some(std::process::id()),
        socket_path: live_sock.clone(),
        session_key: Some("cli:work".into()),
        tab_name: Some("main".into()),
        workspace_id: Some("ws".into()),
        updated_unix_s: 1,
        status: TabAgentStatus::Live,
    });
    reg.store(&rpath).unwrap();

    let manifest = App::test_workspace_manifest(
        "ws",
        vec![WorkspaceTabEntry {
            tab_id: 0,
            session_key: Some("cli:work".into()),
            name: Some("main".into()),
            summary: None,
        }],
        0,
    );
    a.apply_workspace_manifest_with_registry(&manifest, &rpath);

    let master = a.conn_for(TabId::MASTER).unwrap();
    assert_eq!(
        master.socket_path.as_deref(),
        Some(live_sock.as_path()),
        "AC6: connected restore must prefer the live detached socket, not resume into the new master"
    );
    assert!(
        master.pending_attach,
        "live reattach must be scheduled against the detached agent"
    );
    assert!(
        master.pending_session_resume.is_none(),
        "live reattach must not latch a resume_session for the already-running owner"
    );
    assert!(
        fresh_rx.try_recv().is_err(),
        "must not send resume_session into the freshly spawned master while a live owner exists"
    );

    let (reattached, mut reattached_rx) = crate::shell::connection::Connection::live_for_tests();
    a.attach_connection_to_tab(TabId::MASTER, reattached, None);
    let mut wire = Vec::new();
    while let Ok(line) = reattached_rx.try_recv() {
        wire.push(line);
    }
    assert!(
        wire.iter().all(|line| !line.contains("resume_session")),
        "successful live-socket reattach must not send resume_session back to the detached owner: {wire:?}"
    );
}

#[test]
fn resume_key_and_selector_latch_deferred_resume_while_disconnected() {
    let mut a = app();
    a.ac_mut().agent_connected = false;
    a.ac_mut().pending_attach = true;
    a.handle_submit("/resume my-session");
    assert_eq!(
        a.ac().pending_session_resume.as_deref(),
        Some("my-session"),
        "AC5: /resume <key> on a connecting tab must latch deferred resume"
    );

    let mut b = app();
    b.ac_mut().agent_connected = false;
    b.apply_resume_selection("session:sel-key");
    assert_eq!(
        b.ac().pending_session_resume.as_deref(),
        Some("sel-key"),
        "AC5: selector session rows must latch deferred resume when disconnected"
    );
    b.apply_resume_selection("plain-key");
    assert_eq!(
        b.ac().pending_session_resume.as_deref(),
        Some("plain-key"),
        "AC5: bare selector keys must also latch"
    );
}

#[test]
fn apply_workspace_manifest_adopts_workspace_identity_and_label() {
    let mut a = app();
    a.ac_mut().agent_connected = true;
    let original_id = a.workspace_id.clone();
    let mut manifest = App::test_workspace_manifest(
        "ws-resumed",
        vec![WorkspaceTabEntry {
            tab_id: 0,
            session_key: Some("sess".into()),
            name: Some("one".into()),
            summary: None,
        }],
        0,
    );
    manifest.label = "amber-brook".into();
    a.apply_workspace_manifest(&manifest);
    assert_ne!(
        original_id, "ws-resumed",
        "precondition: fresh UUID differs"
    );
    assert_eq!(
        a.workspace_id, "ws-resumed",
        "resume must adopt the manifest workspace id so later persists update \
         the same row instead of forking a duplicate"
    );
    assert_eq!(
        a.workspace_label, "amber-brook",
        "resume must keep the stored label instead of a fresh random one"
    );
}

#[test]
fn apply_workspace_manifest_keeps_generated_label_when_stored_label_empty() {
    let mut a = app();
    a.ac_mut().agent_connected = true;
    let generated = a.workspace_label.clone();
    let manifest = App::test_workspace_manifest(
        "ws-nolabel",
        vec![WorkspaceTabEntry {
            tab_id: 0,
            session_key: Some("sess".into()),
            name: None,
            summary: None,
        }],
        0,
    );
    a.apply_workspace_manifest(&manifest);
    assert_eq!(a.workspace_id, "ws-nolabel");
    assert_eq!(
        a.workspace_label, generated,
        "an empty stored label must not blank the generated one"
    );
}
