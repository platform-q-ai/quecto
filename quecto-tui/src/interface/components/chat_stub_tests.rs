use super::{Chat, ChatEntry};
use crate::interface::component::Component;

fn stub(id: &str) -> ChatEntry {
    ChatEntry::Stub {
        id: id.into(),
        text: format!("stub {id}"),
        is_user: false,
    }
}

#[test]
fn visible_stub_ids_exclude_offscreen_loaded_stubs() {
    let mut chat = Chat::new();
    chat.add_entry(stub("old-offscreen"));
    for i in 0..20 {
        chat.add_entry(ChatEntry::User {
            text: format!("middle {i}"),
        });
    }
    chat.add_entry(stub("new-visible"));
    chat.set_viewport_height(3);
    let _ = chat.render(80);

    assert_eq!(chat.visible_stub_message_ids(), vec!["new-visible"]);

    chat.scroll_up(usize::MAX);
    let _ = chat.render(80);
    assert_eq!(chat.visible_stub_message_ids(), vec!["old-offscreen"]);
}

#[test]
fn stub_role_match_requires_same_message_and_role() {
    let mut chat = Chat::new();
    chat.add_entry(stub("assistant-stub"));
    chat.add_entry(ChatEntry::Stub {
        id: "user-stub".into(),
        text: "user stub".into(),
        is_user: true,
    });

    assert!(chat.stub_role_matches("assistant-stub", "assistant"));
    assert!(!chat.stub_role_matches("assistant-stub", "user"));
    assert!(chat.stub_role_matches("user-stub", "user"));
    assert!(!chat.stub_role_matches("user-stub", "assistant"));
    assert!(!chat.stub_role_matches("missing", "assistant"));
}

#[test]
fn oldest_loaded_boundary_requires_reaching_top_of_long_transcript() {
    let mut chat = Chat::new();
    for i in 0..30 {
        chat.add_entry(ChatEntry::User {
            text: format!("message {i}"),
        });
    }
    chat.set_viewport_height(4);
    let _ = chat.render(80);

    assert!(!chat.is_at_oldest_loaded_history());
    chat.scroll_up(2);
    assert!(!chat.is_at_oldest_loaded_history());

    chat.scroll_up(usize::MAX);
    assert!(chat.is_at_oldest_loaded_history());
}
