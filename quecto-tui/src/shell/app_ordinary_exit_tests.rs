use crate::shell::app::tui_harness::TuiHarness;
use crate::shell::connection::{Connection, TabId};
use crate::shell::keys::Key;

#[tokio::test]
async fn ctrl_d_and_slash_exit_inputs_share_ordinary_exit_request() {
    let mut h = TuiHarness::new().await;
    h.app_mut().handle_key(Key::Ctrl('d'));
    assert!(h.should_exit(), "Ctrl-D requests ordinary exit");
    assert_eq!(
        h.pending_aborts(),
        0,
        "Ctrl-D must not uniquely abort active turns"
    );

    for cmd in ["/exit", "/quit"] {
        let mut h = TuiHarness::new().await;
        h.app_mut().handle_submit(cmd);
        assert!(h.should_exit(), "{cmd} requests ordinary exit");
        assert_eq!(h.pending_aborts(), 0, "{cmd} does not abort while exiting");
    }
}

#[tokio::test]
async fn ordinary_exit_finalizer_persists_all_visible_tabs_before_terminal_cleanup() {
    let mut h = TuiHarness::new().await;
    let a = h.app_mut();
    let (mut master_conn, mut master_rx) = Connection::live_for_tests();
    master_conn.set_tab_for_tests(TabId::MASTER);
    a.attach_connection_to_tab(TabId::MASTER, master_conn, None);
    let tab = a.open_placeholder_tab(Some("worker".into()));
    let (mut tab_conn, mut tab_rx) = Connection::live_for_tests();
    tab_conn.set_tab_for_tests(tab);
    a.attach_connection_to_tab(tab, tab_conn, None);

    a.finalize_ordinary_exit().await;

    let master_cmd: serde_json::Value =
        serde_json::from_str(&master_rx.try_recv().unwrap()).unwrap();
    let tab_cmd: serde_json::Value = serde_json::from_str(&tab_rx.try_recv().unwrap()).unwrap();
    assert_eq!(master_cmd["type"], "persist_session");
    assert_eq!(tab_cmd["type"], "persist_session");
    assert!(a.should_exit);
}
