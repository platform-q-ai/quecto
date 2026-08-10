use super::*;

fn user_entry(text: impl Into<String>) -> ChatEntry {
    ChatEntry::User { text: text.into() }
}

#[test]
fn chat_1196_retention_does_not_evict_at_or_below_cap() {
    let mut chat = Chat::new();
    for i in 0..CHAT_RETAINED_ENTRY_CAP {
        chat.add_entry(user_entry(format!("msg-{i}")));
    }
    assert_eq!(chat.entry_count(), CHAT_RETAINED_ENTRY_CAP);
    assert!(matches!(chat.entries().first(), Some(ChatEntry::User { text }) if text == "msg-0"));

    chat.add_entry(user_entry("overflow"));
    assert!(chat.entry_count() <= CHAT_RETAINED_ENTRY_CAP);
    assert!(
        !chat
            .entries()
            .iter()
            .any(|e| matches!(e, ChatEntry::User { text } if text == "msg-0"))
    );
    assert!(matches!(chat.entries().last(), Some(ChatEntry::User { text }) if text == "overflow"));
}

#[test]
fn chat_1196_retention_cap_applies_to_replace_range_batches() {
    let mut chat = Chat::new();
    chat.add_entry(user_entry("seed"));
    let entries = (0..(CHAT_RETAINED_ENTRY_CAP + 25))
        .map(|i| user_entry(format!("batch-{i}")))
        .collect();
    chat.replace_range(0, 1, entries);
    assert!(chat.entry_count() <= CHAT_RETAINED_ENTRY_CAP);
    assert!(
        !chat
            .entries()
            .iter()
            .any(|e| matches!(e, ChatEntry::User { text } if text == "batch-0"))
    );
    assert!(
        matches!(chat.entries().last(), Some(ChatEntry::User { text }) if text == &format!("batch-{}", CHAT_RETAINED_ENTRY_CAP + 24))
    );
}

#[test]
fn chat_1196_recovered_prepend_is_retained_within_cap() {
    let mut chat = Chat::new();
    for i in 0..(CHAT_RETAINED_ENTRY_CAP - 10) {
        chat.add_entry(user_entry(format!("tail-{i}")));
    }
    chat.prepend_history(vec![user_entry("older-0"), user_entry("older-1")]);
    assert!(chat.entry_count() <= CHAT_RETAINED_ENTRY_CAP);
    assert!(
        chat.entries()
            .iter()
            .any(|e| matches!(e, ChatEntry::User { text } if text == "older-0"))
    );
    assert!(
        chat.entries()
            .iter()
            .any(|e| matches!(e, ChatEntry::User { text } if text == "tail-1013")),
        "history recovery must not evict the live tail"
    );
}

#[test]
fn chat_1196_streaming_survives_eviction_pressure() {
    let mut chat = Chat::new();
    for i in 0..CHAT_RETAINED_ENTRY_CAP {
        chat.add_entry(user_entry(format!("pre-{i}")));
    }
    chat.append_token("stream");
    chat.append_token(" text");
    chat.finalize_assistant();
    assert!(chat.entry_count() <= CHAT_RETAINED_ENTRY_CAP);
    assert_eq!(chat.entries().iter().filter(|e| matches!(e, ChatEntry::Assistant { text, streaming: false } if text == "stream text")).count(), 1);
}

#[test]
fn chat_1196_tool_completion_after_eviction_pressure_updates_by_id_or_safe_noop() {
    let mut chat = Chat::new();
    for i in 0..CHAT_RETAINED_ENTRY_CAP {
        chat.add_entry(user_entry(format!("pre-{i}")));
    }
    chat.start_tool(
        "tc-1196".into(),
        "bash".into(),
        r#"{"command":"echo ok"}"#.into(),
    );
    chat.complete_tool("tc-1196", "ok\n", false, None);
    assert!(chat.entry_count() <= CHAT_RETAINED_ENTRY_CAP);
    assert!(chat.entries().iter().any(|e| matches!(e, ChatEntry::ToolExecution { tool_call_id, result: Some(result), .. } if tool_call_id == "tc-1196" && result == "ok\n")));
}

#[test]
fn chat_1196_large_recovered_prefix_keeps_recent_prefix_and_live_suffix() {
    let mut chat = Chat::new();
    for i in 0..100 {
        chat.add_entry(user_entry(format!("tail-{i}")));
    }
    let older = (0..2000)
        .map(|i| user_entry(format!("older-{i}")))
        .collect();
    chat.prepend_history(older);

    assert_eq!(chat.entry_count(), CHAT_RETAINED_ENTRY_CAP);
    assert!(
        !chat
            .entries()
            .iter()
            .any(|e| matches!(e, ChatEntry::User { text } if text == "older-0"))
    );
    assert!(
        chat.entries()
            .iter()
            .any(|e| matches!(e, ChatEntry::User { text } if text == "older-1999"))
    );
    assert!(
        chat.entries()
            .iter()
            .any(|e| matches!(e, ChatEntry::User { text } if text == "tail-0"))
    );
    assert!(
        chat.entries()
            .iter()
            .any(|e| matches!(e, ChatEntry::User { text } if text == "tail-99"))
    );
}

#[test]
fn chat_1196_prepend_when_suffix_full_does_not_panic_or_drop_newest_suffix() {
    let mut chat = Chat::new();
    for i in 0..CHAT_RETAINED_ENTRY_CAP {
        chat.add_entry(user_entry(format!("tail-{i}")));
    }
    chat.prepend_history(
        (0..2000)
            .map(|i| user_entry(format!("older-{i}")))
            .collect(),
    );

    assert_eq!(chat.entry_count(), CHAT_RETAINED_ENTRY_CAP);
    assert!(
        chat.entries()
            .iter()
            .any(|e| matches!(e, ChatEntry::User { text } if text == "tail-1023"))
    );
    assert!(
        chat.entries()
            .iter()
            .any(|e| matches!(e, ChatEntry::User { text } if text == "older-1999")),
        "a successful recovered page should become visible even when the suffix is full"
    );
}
