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
