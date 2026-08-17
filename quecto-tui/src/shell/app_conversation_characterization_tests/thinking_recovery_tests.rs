use super::*;

#[tokio::test]
async fn recovery_accumulates_all_thinking_pages_before_inserting_message() {
    let mut h = TuiHarness::new().await;
    let batch_id = "batch-thinking-pages".to_string();
    let req1 = "req-thinking-1".to_string();
    let message_id = "msg-thinking".to_string();
    h.app_mut().ac_mut().message_recovery_batches.insert(
        batch_id.clone(),
        MessageRecoveryBatch::new(vec![message_id.clone()], 0, 1, None),
    );
    h.app_mut()
        .ac_mut()
        .master_session
        .chat
        .append_token("stale thinking page placeholder");
    h.app_mut().ac_mut().pending_message_recovery.insert(
        req1.clone(),
        PendingMessageRecovery {
            message_id: message_id.clone(),
            batch_id,
            agent_id: None,
            content: String::new(),
            offset: 0,
            content_len: None,
            thinking: Vec::new(),
            thinking_offset: 0,
        },
    );

    h.app_mut().handle_get_message_recovery(
        Some(&req1),
        true,
        Some(serde_json::json!({
            "id": message_id,
            "role": "assistant",
            "content": "answer",
            "offset": 0,
            "nextOffset": 6,
            "contentLength": 12,
            "hasMoreContent": true,
            "thinking": [{"kind":"text","text":"first "}],
            "thinkingOffset": 0,
            "nextThinkingOffset": 6,
            "thinkingLength": 12,
            "hasMoreThinking": true
        })),
    );
    let follow_up = get_message_commands(&h.drain_commands().await)
        .into_iter()
        .find(|cmd| cmd["thinkingOffset"] == 6)
        .expect("thinking continuation request");
    assert_eq!(follow_up["thinkingOffset"], 6);
    let req2 = follow_up["id"].as_str().unwrap().to_string();

    h.app_mut().handle_get_message_recovery(
        Some(&req2),
        true,
        Some(serde_json::json!({
            "id": message_id,
            "role": "assistant",
            "content": "answer",
            "offset": 6,
            "nextOffset": 12,
            "contentLength": 12,
            "hasMoreContent": false,
            "thinking": [{"kind":"text","text":"second"}],
            "thinkingOffset": 6,
            "nextThinkingOffset": 12,
            "thinkingLength": 12,
            "hasMoreThinking": false
        })),
    );

    let entries = h.app_mut().ac().master_session.chat.entries();
    assert!(entries.iter().any(|entry| matches!(
        entry,
        ChatEntry::Assistant { text, thinking, .. }
            if text == "answeranswer" && thinking.iter().any(|t| t.contains("first second"))
    )));
}

#[tokio::test]
async fn recovery_keeps_redacted_thinking_placeholders() {
    let mut h = TuiHarness::new().await;
    let batch_id = "batch-thinking-redacted".to_string();
    let req1 = "req-thinking-redacted-1".to_string();
    let message_id = "msg-thinking-redacted".to_string();
    h.app_mut().ac_mut().message_recovery_batches.insert(
        batch_id.clone(),
        MessageRecoveryBatch::new(vec![message_id.clone()], 0, 1, None),
    );
    h.app_mut()
        .ac_mut()
        .master_session
        .chat
        .append_token("stale redacted thinking placeholder");
    h.app_mut().ac_mut().pending_message_recovery.insert(
        req1.clone(),
        PendingMessageRecovery {
            message_id: message_id.clone(),
            batch_id,
            agent_id: None,
            content: String::new(),
            offset: 0,
            content_len: None,
            thinking: Vec::new(),
            thinking_offset: 0,
        },
    );

    h.app_mut().handle_get_message_recovery(
        Some(&req1),
        true,
        Some(serde_json::json!({
            "id": message_id,
            "role": "assistant",
            "content": "answer",
            "offset": 0,
            "nextOffset": 6,
            "contentLength": 6,
            "hasMoreContent": false,
            "thinking": [{"kind":"text","text":"λx"}],
            "thinkingOffset": 0,
            "nextThinkingOffset": 2,
            "thinkingLength": 12,
            "hasMoreThinking": true
        })),
    );
    let follow_up = get_message_commands(&h.drain_commands().await)
        .into_iter()
        .find(|cmd| cmd["thinkingOffset"] == 2)
        .expect("thinking continuation request");
    let req2 = follow_up["id"].as_str().unwrap().to_string();

    assert_eq!(follow_up["offset"], 6);
    h.app_mut().handle_get_message_recovery(
        Some(&req2),
        true,
        Some(serde_json::json!({
            "id": message_id,
            "role": "assistant",
            "content": "",
            "offset": 6,
            "nextOffset": 6,
            "contentLength": 6,
            "hasMoreContent": false,
            "thinking": [{"kind":"redacted"}, {"kind":"text","text":"y"}],
            "thinkingOffset": 2,
            "nextThinkingOffset": 5,
            "thinkingLength": 5,
            "hasMoreThinking": false
        })),
    );

    let entries = h.app_mut().ac().master_session.chat.entries();
    assert!(entries.iter().any(|entry| matches!(
        entry,
        ChatEntry::Assistant { text, thinking, .. }
            if text == "answer"
                && thinking == &["λx".to_string(), "[redacted thinking]".to_string(), "y".to_string()]
    )));
}
