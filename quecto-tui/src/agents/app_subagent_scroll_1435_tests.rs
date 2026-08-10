use super::*;
use crate::agents::ledger::LedgerEntry;
use crate::components::chat::ChatEntry;

fn ledger_lines(count: usize) -> Vec<LedgerEntry> {
    (0..count)
        .map(|i| LedgerEntry::User {
            text: format!("history line {i}"),
        })
        .collect()
}

#[test]
fn selected_session_ledger_projection_preserves_scrolled_history_viewport() {
    let mut session = SessionView::new(None);
    session.project_ledger_with_live(ledger_lines(30), false, false);
    session.chat.set_viewport_height(10);
    session.chat.scroll_up(15);
    let before = session.chat.render(80);

    let mut updated = ledger_lines(30);
    updated.push(LedgerEntry::Assistant {
        text: "new live ledger content".into(),
    });
    session.project_ledger_with_live(updated, false, false);
    let after = session.chat.render(80);

    assert_eq!(
        after, before,
        "ledger/feed refresh must not reset an intentionally scrolled subagent transcript"
    );
}

#[test]
fn selected_session_completion_status_preserves_scrolled_history_viewport() {
    let mut session = SessionView::new(None);
    for entry in ledger_lines(30) {
        session
            .chat
            .add_entry(crate::agents::view::ledger_entry_to_chat_entry(entry));
    }
    session.chat.set_viewport_height(10);
    session.chat.scroll_up(15);
    let before = session.chat.render(80);

    session.chat.add_entry(ChatEntry::Status {
        text: "subagent completed".into(),
    });
    let after = session.chat.render(80);

    assert_eq!(
        after, before,
        "completion/status entries must not snap a scrolled subagent transcript to the tail"
    );
}

#[test]
fn selected_session_returning_to_tail_restores_live_following() {
    let mut session = SessionView::new(None);
    session.project_ledger_with_live(ledger_lines(30), false, false);
    session.chat.set_viewport_height(10);
    session.chat.scroll_up(15);
    let _ = session.chat.render(80);

    session.chat.scroll_down(usize::MAX);
    session.chat.add_entry(ChatEntry::Status {
        text: "new tail status".into(),
    });
    let after = session.chat.render(80).join("\n");

    assert!(
        after.contains("new tail status"),
        "once returned to tail, subsequent subagent updates should remain visible"
    );
    assert_eq!(session.chat.scroll_offset(), 0);
}

#[tokio::test]
async fn handle_submit_master_message_repins_scrolled_chat_to_tail() {
    let mut h = crate::shell::app::tui_harness::TuiHarness::new().await;
    let a = h.app_mut();
    for i in 0..30 {
        a.master_session.chat.add_entry(ChatEntry::User {
            text: format!("history line {i}"),
        });
    }
    a.master_session.chat.set_viewport_height(10);
    a.master_session.chat.scroll_up(15);
    let _ = a.master_session.chat.render(80);

    a.handle_submit("visible master prompt");
    let after = a.master_session.chat.render(80).join("\n");

    assert_eq!(a.master_session.chat.scroll_offset(), 0);
    assert!(after.contains("visible master prompt"));
}

#[tokio::test]
async fn handle_submit_subagent_message_repins_scrolled_chat_to_tail() {
    let mut h = crate::shell::app::tui_harness::TuiHarness::new().await;
    let a = h.app_mut();
    let (tx, _rx) = tokio::sync::mpsc::channel(4);
    let id = "child".to_string();
    a.subagents.active_agent_id = Some(id.clone());
    a.subagents.feeds.insert(
        id.clone(),
        crate::agents::view::FeedState {
            cmd_tx: tx,
            handle: tokio::spawn(async {}),
            inspection_only: false,
            epoch: 0,
            rev: 0,
            last_fresh_at: None,
            supports_sync: true,
            pending_rev: None,
            transcript: crate::agents::ledger::LedgerTranscript::default(),
            authority: crate::agents::feed::FeedAuthority::WarmSync,
        },
    );
    a.ensure_session(&id);
    let session = a.subagents.sessions.get_mut(&id).unwrap();
    for i in 0..30 {
        session.chat.add_entry(ChatEntry::User {
            text: format!("history line {i}"),
        });
    }
    session.chat.set_viewport_height(10);
    session.chat.scroll_up(15);
    let _ = session.chat.render(80);

    a.handle_submit("visible child prompt");
    let chat = &mut a.subagents.sessions.get_mut(&id).unwrap().chat;
    let after = chat.render(80).join("\n");

    assert_eq!(chat.scroll_offset(), 0);
    assert!(after.contains("visible child prompt"));
}
