use super::*;

// --- normalize_messages clone-on-write tests (#374) ---

#[test]
fn test_normalize_messages_does_not_clone_unmodified_messages() {
    // Messages with no tool calls need no normalization — they should be
    // returned as borrowed Cow::Borrowed, not deep-cloned.
    let messages = vec![
        Message::user("hello"),
        Message::assistant("world", vec![]),
        Message::user("follow up"),
    ];

    let normalized = AnthropicProvider::normalize_messages(&messages);
    assert_eq!(normalized.len(), 3);

    // Verify pointer equality: each Cow::Borrowed should point to the
    // original message, not a clone.
    for (i, cow) in normalized.iter().enumerate() {
        let original_ptr = &messages[i] as *const Message;
        // Cow::Borrowed derefs to the original; Cow::Owned derefs to a new alloc.
        let normalized_ptr: *const Message = &**cow;
        assert_eq!(
            original_ptr, normalized_ptr,
            "message {} should be Cow::Borrowed (same pointer), not cloned",
            i
        );
    }
}
