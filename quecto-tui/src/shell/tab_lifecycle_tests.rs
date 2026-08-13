use crate::protocol::client::Client;
use crate::shell::app::App;
use crate::shell::connection::TabId;
use crate::shell::tab_registry::TabAgentRegistry;
use crate::shell::terminal::Terminal;
use crate::shell::workspace_manifest::WorkspaceManifestStore;

fn app() -> App {
    let client = Client::disconnected_for_tests();
    let mut term = Terminal::new();
    term.set_size_for_tests(80, 24);
    App::new(term, client)
}

#[test]
fn open_placeholder_allocates_and_focuses() {
    let mut a = app();
    assert_eq!(a.tabs.len(), 1);
    let t1 = a.open_placeholder_tab(Some("two".into()));
    assert_eq!(t1, TabId(1));
    assert_eq!(a.tabs.len(), 2);
    assert_eq!(a.active_tab, TabId(1));
    assert_eq!(a.ac().name.as_deref(), Some("two"));
    assert!(!a.ac().agent_connected);
}

#[test]
fn switch_tab_next_prev_wraps() {
    let mut a = app();
    let _ = a.open_placeholder_tab(None);
    let _ = a.open_placeholder_tab(None);
    assert_eq!(a.active_tab, TabId(2));
    assert_eq!(a.switch_tab_prev(), TabId(1));
    assert_eq!(a.switch_tab_prev(), TabId(0));
    assert_eq!(a.switch_tab_prev(), TabId(2));
    assert_eq!(a.switch_tab_next(), TabId(0));
}

#[test]
fn close_tab_detaches_and_refocuses() {
    let mut a = app();
    let t1 = a.open_placeholder_tab(None);
    let watch = a.close_tab(t1, false).unwrap();
    assert!(watch.is_none());
    assert_eq!(a.tabs.len(), 1);
    assert_eq!(a.active_tab, TabId::MASTER);
}

#[test]
fn close_last_tab_refused() {
    let mut a = app();
    assert!(a.close_tab(TabId::MASTER, false).is_err());
}

#[test]
fn close_active_prefers_previous_id() {
    let mut a = app();
    let _t1 = a.open_placeholder_tab(None);
    let t2 = a.open_placeholder_tab(None);
    assert_eq!(a.active_tab, t2);
    a.close_tab(t2, false).unwrap();
    assert_eq!(a.active_tab, TabId(1));
}

#[test]
fn registry_and_manifest_snapshots_track_tabs() {
    let mut a = app();
    a.open_placeholder_tab(Some("b".into()));
    let reg = a.registry_snapshot(Some("ws"));
    assert_eq!(reg.agents.len(), 2);
    assert_eq!(reg.agents[1].tab_name.as_deref(), Some("b"));
    let man = a.workspace_manifest_snapshot("ws");
    assert_eq!(man.tabs.len(), 2);
    assert_eq!(man.active_index, 1);

    let dir = tempfile::tempdir().unwrap();
    let rpath = dir.path().join("r.json");
    let mpath = dir.path().join("m.json");
    a.persist_durability_snapshot("ws", &rpath, &mpath);
    let loaded_r = TabAgentRegistry::load(&rpath);
    assert_eq!(loaded_r.agents.len(), 2);
    let loaded_m = WorkspaceManifestStore::load(&mpath);
    assert_eq!(loaded_m.get("ws").unwrap().tabs.len(), 2);
}

#[test]
fn switch_unknown_tab_is_false() {
    let mut a = app();
    assert!(!a.switch_tab(TabId(9)));
    assert_eq!(a.active_tab, TabId::MASTER);
}

#[test]
fn attach_connection_marks_tab_connected() {
    let mut a = app();
    let tab = a.open_placeholder_tab(None);
    assert!(!a.conn_for(tab).unwrap().agent_connected);
    let conn = crate::shell::connection::Connection::placeholder(tab);
    a.attach_connection_to_tab(tab, conn, None);
    assert!(a.conn_for(tab).unwrap().agent_connected);
    assert_eq!(a.active_tab, tab);
}
