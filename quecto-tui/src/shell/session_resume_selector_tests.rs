use crate::protocol::client::Client;
use crate::shell::app::App;
use crate::shell::terminal::Terminal;

fn app() -> App {
    let client = Client::disconnected_for_tests();
    let mut term = Terminal::new();
    term.set_size_for_tests(80, 24);
    App::new(term, client)
}

#[test]
fn bare_session_selection_resumes_in_the_current_session() {
    let mut a = app();
    // Live writer so resume commands are observable on the connection's sender.
    let (live, mut rx) = crate::shell::connection::Connection::live_for_tests();
    a.ac_mut().transport = live;
    a.ac_mut().agent_connected = true;
    a.apply_resume_selection("session:my-key");
    let line = rx
        .try_recv()
        .expect("session: prefix must send resume on the connection");
    assert!(line.contains("resume_session"), "wire={line}");
    assert!(line.contains("my-key"), "wire={line}");
    a.apply_resume_selection("plain-key");
    let line = rx
        .try_recv()
        .expect("bare key must send resume on the connection");
    assert!(line.contains("resume_session"), "wire={line}");
    assert!(line.contains("plain-key"), "wire={line}");
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
