//! Two TUI clients attached to ONE harness (#2044): every answer is broadcast
//! to both, both mint ids under the same constant prefix, and an answer is a
//! client's own only when its id equals that client's pending id.

use super::tui_harness::TuiHarness;
use crate::protocol::client::Event;

/// Submit `/resume <key>` and return the id the client put on the wire.
async fn typed_resume(h: &mut TuiHarness, key: &str) -> String {
    h.app_mut().handle_submit(&format!("/resume {key}"));
    let id = h
        .app_mut()
        .ac()
        .pending_session_resume_id
        .clone()
        .expect("a typed resume is in flight");
    let sent = h.drain_commands().await;
    assert!(
        sent.iter()
            .any(|line| line.contains("resume_session") && line.contains(&id)),
        "the pending id is the one on the wire: {sent:?}"
    );
    id
}

/// The harness refuses `id`; the broadcast reaches both clients.
fn broadcast_refusal(clients: [&mut TuiHarness; 2], id: &str) {
    for client in clients {
        client.app_mut().handle_event(Event::Response {
            id: Some(id.to_string()),
            command: "resume_session".into(),
            success: false,
            data: None,
            error: Some("no such session".into()),
        });
    }
}

#[tokio::test]
async fn a_refused_resume_is_told_only_to_the_client_that_asked() {
    let mut a = TuiHarness::new().await;
    let mut b = TuiHarness::new().await;
    let a_id = typed_resume(&mut a, "ghost-a").await;
    let b_id = typed_resume(&mut b, "ghost-b").await;
    assert_ne!(a_id, b_id, "two clients never mint the same resume id");
    assert_eq!(
        a_id.split(':').next(),
        b_id.split(':').next(),
        "both clients mint under the same prefix: it cannot tell them apart"
    );

    broadcast_refusal([&mut a, &mut b], &a_id);

    assert!(
        a.notification_messages()
            .iter()
            .any(|m| m.contains("Resume failed")),
        "the asker is told: {:?}",
        a.notification_messages()
    );
    assert_eq!(a.app_mut().ac().pending_session_resume_id, None);
    assert_eq!(
        b.notification_messages(),
        Vec::<String>::new(),
        "a peer's refusal shows nothing here"
    );
    assert_eq!(
        b.app_mut().ac().pending_session_resume_id.as_deref(),
        Some(b_id.as_str()),
        "a peer's answer leaves this client's resume in flight"
    );

    // Mirrored: B's own refusal settles B and adds nothing to A.
    let a_before = a.notification_messages();
    broadcast_refusal([&mut a, &mut b], &b_id);

    assert!(
        b.notification_messages()
            .iter()
            .any(|m| m.contains("Resume failed")),
        "the asker is told: {:?}",
        b.notification_messages()
    );
    assert_eq!(b.app_mut().ac().pending_session_resume_id, None);
    assert_eq!(a.notification_messages(), a_before);
}
