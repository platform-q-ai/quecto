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
fn resume_key_and_selector_latch_deferred_resume_while_disconnected() {
    let mut a = app();
    a.ac_mut().agent_connected = false;
    a.handle_submit("/resume my-session");
    assert_eq!(
        a.ac().pending_session_resume.as_deref(),
        Some("my-session"),
        "AC5: /resume <key> on a disconnected connection must latch deferred resume"
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
