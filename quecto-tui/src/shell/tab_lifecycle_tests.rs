use crate::protocol::client::Client;
use crate::shell::app::App;
use crate::shell::connection::TabId;
use crate::shell::terminal::Terminal;

fn app() -> App {
    let client = Client::disconnected_for_tests();
    let mut term = Terminal::new();
    term.set_size_for_tests(80, 24);
    App::new(term, client)
}

#[test]
fn new_command_resets_workspace_not_just_active_session() {
    let mut a = app();
    a.active_chat_mut()
        .add_entry(crate::components::chat::ChatEntry::User {
            text: "master old".into(),
        });
    let t1 = a.test_open_disconnected_tab();
    a.test_set_active_tab(t1.0);
    a.active_chat_mut()
        .add_entry(crate::components::chat::ChatEntry::User {
            text: "background old".into(),
        });
    a.subagents.panel_nav_key = Some("stale-agent".into());
    a.subagents.panel_nav.set_selected(3);

    a.handle_submit("/new");

    assert_eq!(a.tabs.len(), 1, "/new must close stale workspace tabs");
    assert!(a.tabs.contains_key(&TabId::MASTER));
    assert!(!a.tabs.contains_key(&t1));
    assert_eq!(a.active_tab, TabId::MASTER);
    assert_eq!(a.ac().master_session.chat.entry_count(), 0);
    assert_eq!(a.ac().name, None);
    assert_eq!(a.ac().session_key, None);
    assert_eq!(a.ac().pending_session_resume, None);
    assert_eq!(a.subagents.panel_nav_key, None);
    assert_eq!(a.subagents.panel_nav.selected(), 0);
}

#[test]
fn reset_workspace_returns_stale_tab_child_watches_for_termination() {
    let mut a = app();
    let t1 = a.test_open_disconnected_tab();
    a.conn_mut(t1).unwrap().child_exit_watch =
        Some(crate::shell::child_watch::ChildWatch::for_tests(Some(77)));

    let watches = a.reset_workspace();

    assert_eq!(
        watches.iter().map(|w| w.pid()).collect::<Vec<_>>(),
        vec![Some(77)],
        "/new must hand back non-master ChildWatch values so callers can terminate owned agents"
    );
    assert_eq!(a.tabs.len(), 1);
    assert!(a.tabs.contains_key(&TabId::MASTER));
}

#[test]
fn removed_tab_slash_commands_are_unknown_and_do_not_mutate_tabs() {
    for command in ["/tab-new", "/tab-close", "/tab-next", "/tab-prev"] {
        let mut a = app();
        let extra = a.test_open_disconnected_tab();
        a.test_set_active_tab(extra.0);
        let before_active = a.active_tab;
        a.handle_submit(command);
        assert_eq!(a.tabs.len(), 2, "{command} must not open/close tabs");
        assert_eq!(
            a.active_tab, before_active,
            "{command} must not switch tabs"
        );
        let has_unknown_status = a.ac().master_session.chat.entries().iter().any(|entry| {
            matches!(entry, crate::components::chat::ChatEntry::Status { text } if text.contains("Unknown slash command"))
        });
        assert!(
            has_unknown_status,
            "{command} should use ordinary unknown-command UX"
        );
    }
}

#[test]
fn collect_owned_child_watches_includes_every_tab() {
    let mut a = app();
    let t1 = a.test_open_disconnected_tab();
    a.conn_mut(TabId::MASTER).unwrap().child_exit_watch =
        Some(crate::shell::child_watch::ChildWatch::for_tests(Some(1)));
    a.conn_mut(t1).unwrap().child_exit_watch =
        Some(crate::shell::child_watch::ChildWatch::for_tests(Some(2)));
    let watches = a.take_all_child_exit_watches();
    assert_eq!(watches.len(), 2, "kill-on-exit must see every tab watch");
}

#[test]
fn tab_commands_are_absent_from_help_registry() {
    let cmds = super::super::builtin_commands();
    for removed in ["tab-new", "tab-close", "tab-next", "tab-prev"] {
        assert!(
            cmds.iter().all(|c| c.name != removed),
            "removed tab command {removed} must not appear in help/autocomplete registry"
        );
    }
}

#[test]
fn ordinary_exit_fanout_targets_all_sendable_tabs_without_focus_or_name_collapse() {
    let mut a = app();
    let (mut master_conn, mut master_rx) = crate::shell::connection::Connection::live_for_tests();
    master_conn.set_tab_for_tests(TabId::MASTER);
    a.test_attach_connection(TabId::MASTER, master_conn, None);

    let t1 = a.test_open_disconnected_tab();
    let (mut tab_conn, mut tab_rx) = crate::shell::connection::Connection::live_for_tests();
    tab_conn.set_tab_for_tests(t1);
    a.test_attach_connection(t1, tab_conn, None);
    let t2 = a.test_open_disconnected_tab();
    let (mut tab2_conn, mut tab2_rx) = crate::shell::connection::Connection::live_for_tests();
    tab2_conn.set_tab_for_tests(t2);
    a.test_attach_connection(t2, tab2_conn, None);

    a.enqueue_ordinary_exit_snapshot_persists().unwrap();

    let master_cmd: serde_json::Value =
        serde_json::from_str(&master_rx.try_recv().unwrap()).unwrap();
    let tab_cmd: serde_json::Value = serde_json::from_str(&tab_rx.try_recv().unwrap()).unwrap();
    let tab2_cmd: serde_json::Value = serde_json::from_str(&tab2_rx.try_recv().unwrap()).unwrap();
    assert_eq!(master_cmd["type"], "persist_session");
    assert_eq!(tab_cmd["type"], "persist_session");
    assert_eq!(tab2_cmd["type"], "persist_session");
    assert!(master_cmd["restoreReason"].is_null());
    assert!(tab_cmd["restoreReason"].is_null());
    assert!(tab2_cmd["restoreReason"].is_null());
    assert_ne!(master_cmd["id"], tab_cmd["id"]);
    assert_ne!(tab_cmd["id"], tab2_cmd["id"]);
    assert!(master_cmd["id"].as_str().unwrap().starts_with("tab0:"));
    assert!(tab_cmd["id"].as_str().unwrap().starts_with("tab1:"));
    assert!(tab2_cmd["id"].as_str().unwrap().starts_with("tab2:"));
    assert_eq!(a.active_tab, TabId::MASTER);
}

#[test]
fn ordinary_exit_fanout_continues_after_first_enqueue_failure() {
    let mut a = app();
    a.test_attach_connection(
        TabId::MASTER,
        crate::shell::connection::Connection::disconnected_for_tests(),
        None,
    );

    let t1 = a.test_open_disconnected_tab();
    let (mut tab_conn, mut tab_rx) = crate::shell::connection::Connection::live_for_tests();
    tab_conn.set_tab_for_tests(t1);
    a.test_attach_connection(t1, tab_conn, None);

    let err = a.enqueue_ordinary_exit_snapshot_persists().unwrap_err();

    let (successful_ids, err) = err;
    assert_eq!(
        successful_ids.len(),
        1,
        "successful enqueue id is returned for barrier waiting"
    );
    assert!(matches!(
        err,
        crate::protocol::client::ClientError::Disconnected
    ));
    let tab_cmd: serde_json::Value = serde_json::from_str(&tab_rx.try_recv().unwrap()).unwrap();
    assert_eq!(tab_cmd["type"], "persist_session");
    assert!(tab_cmd["restoreReason"].is_null());
}

#[test]
fn ordinary_exit_persistence_distinguishes_owned_killing_from_detach_and_external() {
    for (owned, kill_owned) in [(true, true), (true, false), (false, true)] {
        let mut a = app();
        a.set_ordinary_exit_kill_owned(kill_owned);
        let (mut conn, mut rx) = crate::shell::connection::Connection::live_for_tests();
        conn.set_tab_for_tests(TabId::MASTER);
        let watch = owned.then(|| crate::shell::child_watch::ChildWatch::for_tests(Some(123)));
        a.test_attach_connection(TabId::MASTER, conn, watch);
        a.enqueue_ordinary_exit_snapshot_persists().unwrap();
        let cmd: serde_json::Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
        assert_eq!(cmd["type"], "persist_session");
        if owned && kill_owned {
            assert_eq!(cmd["restoreReason"], "ordinary_tui_exit_stopped");
        } else {
            assert!(
                cmd["restoreReason"].is_null(),
                "detach/external exit must preserve live recovery"
            );
        }
    }
}

/// #1956 review: `/new` terminates only the non-master tabs' watches — the
/// master's own watch stays with the surviving master tab.
#[test]
fn reset_workspace_leaves_the_master_watch_and_takes_only_other_tabs() {
    let mut a = app();
    a.conn_mut(TabId::MASTER).unwrap().child_exit_watch =
        Some(crate::shell::child_watch::ChildWatch::for_tests(Some(1)));
    let t1 = a.test_open_disconnected_tab();
    let t2 = a.test_open_disconnected_tab();
    a.conn_mut(t1).unwrap().child_exit_watch =
        Some(crate::shell::child_watch::ChildWatch::for_tests(Some(2)));
    a.conn_mut(t2).unwrap().child_exit_watch =
        Some(crate::shell::child_watch::ChildWatch::for_tests(Some(3)));

    let mut taken: Vec<_> = a.reset_workspace().iter().map(|w| w.pid()).collect();
    taken.sort();

    assert_eq!(taken, vec![Some(2), Some(3)]);
    assert_eq!(
        a.conn_for(TabId::MASTER)
            .unwrap()
            .child_exit_watch
            .as_ref()
            .and_then(|w| w.pid()),
        Some(1),
        "the master's watch is not handed out for termination"
    );
}
