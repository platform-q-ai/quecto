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
fn a_disconnected_resume_is_refused_aloud_and_leaves_nothing_in_flight() {
    // The TUI never reconnects, so nothing is latched for later (#2044): the
    // typed key and the picker row are both refused by `send_command`.
    let mut a = app();
    a.ac_mut().agent_connected = false;
    a.handle_submit("/resume my-session");
    assert_eq!(a.ac().pending_session_resume_id, None);
    assert!(
        a.ac().disconnect_refusal_notified,
        "/resume <key> on a dead connection must tell the user it was not sent"
    );

    let mut b = app();
    b.ac_mut().agent_connected = false;
    b.apply_resume_selection("session:sel-key");
    assert_eq!(b.ac().pending_session_resume_id, None);
    assert!(
        b.ac().disconnect_refusal_notified,
        "a picker row chosen on a dead connection must tell the user it was not sent"
    );
}
