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
            },
            WorkspaceTabEntry {
                tab_id: 1,
                session_key: Some("dead-sess".into()),
                name: Some("two".into()),
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
    a.ac_mut().agent_connected = true;
    a.apply_resume_selection("session:my-key");
    a.apply_resume_selection("plain-key");
}

#[test]
fn unknown_workspace_notifies_without_changing_tabs() {
    let mut a = app();
    let n = a.tabs.len();
    a.restore_workspace_from_path(
        "nope",
        std::path::Path::new("/tmp/does-not-exist-1465.json"),
    );
    assert_eq!(a.tabs.len(), n);
}
