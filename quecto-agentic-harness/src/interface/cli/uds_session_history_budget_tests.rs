use super::super::messages_page_json;
use super::{HISTORY_MESSAGE_SUMMARY_PREVIEW_BYTES, HISTORY_PAGE_JSON_BUDGET};
use crate::domain::message::{Message, ToolCall};

fn message_content<'a>(message: &'a serde_json::Value, field: &str) -> &'a str {
    message
        .get(field)
        .and_then(|value| value.as_str())
        .expect("history message should include requested string field")
}

fn page_messages(page: &serde_json::Value) -> &[serde_json::Value] {
    page.get("messages")
        .and_then(|value| value.as_array())
        .map(Vec::as_slice)
        .expect("history page should include messages array")
}

#[test]
fn get_messages_summarises_an_oversized_message_and_keeps_the_page_bounded() {
    let oversized_body = "oversized-output".repeat(HISTORY_PAGE_JSON_BUDGET / 8);
    let messages = vec![
        Message::user("omitted oldest reachable message"),
        Message::user("included older message"),
        Message::assistant(oversized_body.clone(), vec![]),
    ];

    let page = messages_page_json(&messages, 2, None);
    let encoded = serde_json::to_vec(&page).expect("history page serializes");
    let page_messages = page_messages(&page);
    let oversized = page_messages
        .iter()
        .find(|message| message.get("role").and_then(|value| value.as_str()) == Some("assistant"))
        .expect("oversized assistant message should remain present as a summary");

    assert!(
        encoded.len() <= HISTORY_PAGE_JSON_BUDGET,
        "get_messages page should stay within the history byte budget: {} > {}; page={page}",
        encoded.len(),
        HISTORY_PAGE_JSON_BUDGET
    );
    assert_eq!(
        oversized.get("collapsed").and_then(|value| value.as_bool()),
        Some(true),
        "oversized history message should be marked as a summary: {oversized}"
    );
    assert_eq!(
        oversized.get("truncated").and_then(|value| value.as_bool()),
        Some(true),
        "oversized history message should advertise truncation: {oversized}"
    );
    assert_eq!(
        oversized
            .get("contentLength")
            .and_then(|value| value.as_u64()),
        Some(oversized_body.len() as u64),
        "summary should carry the byte length needed for ranged get_message recovery: {oversized}"
    );
    assert!(
        !message_content(oversized, "content").is_empty()
            && message_content(oversized, "content").len() < oversized_body.len(),
        "summary should include only a bounded preview, not the full oversized body"
    );
    assert!(
        oversized
            .get("id")
            .and_then(|value| value.as_str())
            .is_some(),
        "summary should carry a stable message id for get_message recovery: {oversized}"
    );
    assert_eq!(
        page.get("hasMoreBefore").and_then(|value| value.as_bool()),
        Some(true),
        "omitted older history should remain reachable through the before cursor: {page}"
    );
    assert!(
        page.get("before")
            .and_then(|value| value.as_str())
            .is_some(),
        "bounded page with omitted older history should carry a before cursor: {page}"
    );
}

#[test]
fn history_summary_boundary_is_pinned_on_both_sides() {
    let complete_body = "a".repeat(HISTORY_PAGE_JSON_BUDGET / 4);
    let summarised_body = "b".repeat(HISTORY_PAGE_JSON_BUDGET);

    let complete_page = messages_page_json(
        &[Message::assistant(complete_body.clone(), vec![])],
        1,
        None,
    );
    let summarised_page = messages_page_json(
        &[Message::assistant(summarised_body.clone(), vec![])],
        1,
        None,
    );
    let complete = &page_messages(&complete_page)[0];
    let summarised = &page_messages(&summarised_page)[0];

    assert_eq!(message_content(complete, "content"), complete_body);
    assert_eq!(
        complete.get("collapsed").and_then(|value| value.as_bool()),
        Some(false),
        "content well inside the page budget should remain complete: {complete}"
    );
    assert_eq!(
        summarised
            .get("collapsed")
            .and_then(|value| value.as_bool()),
        Some(true),
        "content beyond the page budget should be summarised: {summarised}"
    );
    assert_eq!(
        message_content(summarised, "content").len(),
        HISTORY_MESSAGE_SUMMARY_PREVIEW_BYTES,
        "summary preview should be capped at the preview boundary"
    );
    assert_eq!(
        summarised
            .get("contentLength")
            .and_then(|value| value.as_u64()),
        Some(summarised_body.len() as u64)
    );
}

#[test]
fn get_messages_summarises_messages_with_oversized_tool_calls() {
    let oversized_arguments =
        "{\"payload\":\"".to_string() + &"t".repeat(HISTORY_PAGE_JSON_BUDGET) + "\"}";
    let arguments_len = oversized_arguments.len();
    let message = Message::assistant(
        "small assistant content",
        vec![ToolCall {
            id: "call-huge".into(),
            name: "huge_tool".into(),
            arguments: oversized_arguments,
        }],
    );

    let page = messages_page_json(&[message], 1, None);
    let encoded = serde_json::to_vec(&page).expect("history page serializes");
    let summary = &page_messages(&page)[0];

    assert!(
        encoded.len() <= HISTORY_PAGE_JSON_BUDGET,
        "tool-call summary should stay within budget: {} > {}; page={page}",
        encoded.len(),
        HISTORY_PAGE_JSON_BUDGET
    );
    assert_eq!(
        summary.get("collapsed").and_then(|value| value.as_bool()),
        Some(true)
    );
    let tool_calls = summary["toolCalls"]
        .as_array()
        .expect("tool-call summaries");
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0]["id"], "call-huge");
    assert_eq!(tool_calls[0]["name"], "huge_tool");
    assert_eq!(tool_calls[0]["arguments"], "");
    assert_eq!(tool_calls[0]["argumentsLength"], arguments_len);
    assert_eq!(tool_calls[0]["truncated"], true);
    assert!(
        summary.get("id").and_then(|value| value.as_str()).is_some(),
        "summary should carry a stable id so get_message can recover full tool calls: {summary}"
    );
}

#[test]
fn get_messages_keeps_small_messages_complete_and_unstubbed() {
    let messages = vec![
        Message::user("first small message"),
        Message::assistant("second small message", vec![]),
    ];

    let page = messages_page_json(&messages, 2, None);
    let page_messages = page_messages(&page);

    assert_eq!(
        page_messages.len(),
        2,
        "small page should keep the requested messages"
    );
    assert_eq!(
        message_content(&page_messages[0], "content"),
        "first small message"
    );
    assert_eq!(
        message_content(&page_messages[1], "content"),
        "second small message"
    );
    assert!(
        page_messages.iter().all(|message| {
            message.get("collapsed").and_then(|value| value.as_bool()) == Some(false)
                && message.get("truncated").is_none()
                && message.get("contentLength").is_none()
        }),
        "small messages should not be represented as recovery summaries: {page}"
    );
}

#[test]
fn collapsed_history_summary_preserves_visible_thinking() {
    use crate::domain::message::{Message, ThinkingBlock};
    let mut msg = Message::assistant("x".repeat(super::HISTORY_PAGE_JSON_BUDGET), vec![]);
    msg.thinking_blocks.push(ThinkingBlock::Normal {
        thinking: "visible reasoning".into(),
        signature: "private".into(),
    });

    let summary = super::message_to_json_for_history_page(&msg);

    assert_eq!(summary["collapsed"], true);
    assert_eq!(summary["thinking"][0]["kind"], "text");
    assert_eq!(summary["thinking"][0]["text"], "visible reasoning");
    assert!(!serde_json::to_string(&summary).unwrap().contains("private"));
}

#[test]
fn collapsed_history_summary_bounds_oversized_visible_thinking() {
    use crate::domain::message::{Message, ThinkingBlock};
    let huge_reasoning = "visible reasoning ".repeat(super::HISTORY_PAGE_JSON_BUDGET / 4);
    let mut msg = Message::assistant("x".repeat(super::HISTORY_PAGE_JSON_BUDGET), vec![]);
    msg.thinking_blocks.push(ThinkingBlock::Normal {
        thinking: huge_reasoning.clone(),
        signature: "private".into(),
    });

    let page = messages_page_json(&[msg], 1, None);
    let encoded = serde_json::to_vec(&page).expect("history page serializes");
    let summary = &page_messages(&page)[0];

    assert!(
        encoded.len() <= HISTORY_PAGE_JSON_BUDGET,
        "collapsed summary with reasoning should stay within budget: {} > {}; page={page}",
        encoded.len(),
        HISTORY_PAGE_JSON_BUDGET
    );
    assert_eq!(summary["collapsed"], true);
    assert_eq!(summary["thinking"][0]["kind"], "text");
    let text = summary["thinking"][0]["text"]
        .as_str()
        .expect("thinking text should remain visible");
    assert!(
        !text.is_empty() && text.len() < huge_reasoning.len(),
        "oversized visible thinking should be previewed, not emitted in full"
    );
    assert_eq!(summary["thinking"][0]["truncated"], true);
    assert_eq!(
        summary["thinking"][0]["textLength"].as_u64(),
        Some(huge_reasoning.len() as u64)
    );
    assert!(!serde_json::to_string(summary).unwrap().contains("private"));
}

#[test]
fn collapsed_history_summary_bounds_many_visible_thinking_blocks() {
    use crate::domain::message::{Message, ThinkingBlock};
    let mut msg = Message::assistant("x".repeat(super::HISTORY_PAGE_JSON_BUDGET), vec![]);
    for idx in 0..(super::HISTORY_PAGE_JSON_BUDGET / 64) {
        msg.thinking_blocks.push(ThinkingBlock::Normal {
            thinking: format!("block-{idx}-{}", "visible reasoning ".repeat(64)),
            signature: "private".into(),
        });
    }

    let page = messages_page_json(&[msg], 1, None);
    let encoded = serde_json::to_vec(&page).expect("history page serializes");
    let summary = &page_messages(&page)[0];

    assert!(
        encoded.len() <= HISTORY_PAGE_JSON_BUDGET,
        "collapsed summary with many thinking blocks should stay within budget: {} > {}; page={page}",
        encoded.len(),
        HISTORY_PAGE_JSON_BUDGET
    );
    assert_eq!(summary["collapsed"], true);
    let thinking = summary["thinking"]
        .as_array()
        .expect("bounded summary should retain a thinking array");
    assert!(
        !thinking.is_empty(),
        "bounded summary should preserve at least one visible thinking preview: {summary}"
    );
    assert!(
        thinking.len() < super::HISTORY_PAGE_JSON_BUDGET / 64,
        "bounded summary should cap the number of thinking blocks: {summary}"
    );
    assert!(
        thinking
            .iter()
            .any(|block| block["truncated"].as_bool() == Some(true)),
        "bounded summary should mark omitted/truncated thinking: {summary}"
    );
    assert!(!serde_json::to_string(summary).unwrap().contains("private"));
}

/// When the byte budget drops older messages of the window, the page still
/// reports older history and its cursor names the oldest KEPT message, so a
/// client pages on from exactly where the frame stopped (#1971).
#[test]
fn budget_trimming_reports_more_history_and_moves_the_cursor_to_the_oldest_kept() {
    let body = "b".repeat(HISTORY_PAGE_JSON_BUDGET / 3);
    let messages: Vec<Message> = (0..6)
        .map(|i| Message::user(format!("{i}-{body}")))
        .collect();
    // The whole window is requested; only the newest two fit the budget.
    let page = messages_page_json(&messages, 6, None);
    let kept = page_messages(&page);
    assert!(
        kept.len() < 6 && !kept.is_empty(),
        "the budget must drop some but not all of the window: {}",
        kept.len()
    );
    assert_eq!(
        message_content(kept.last().unwrap(), "content"),
        format!("5-{body}"),
        "the newest message always arrives"
    );
    assert_eq!(page["hasMoreBefore"], true);
    assert_eq!(
        page["before"], kept[0]["id"],
        "the cursor names the oldest kept message, not the oldest of the window"
    );
    let encoded = serde_json::to_vec(&page).unwrap();
    assert!(encoded.len() <= HISTORY_PAGE_JSON_BUDGET);
}

/// #2162: an encrypted reasoning item is for the provider only: neither a
/// full history message nor a summarised one shows it.
#[test]
fn history_never_shows_encrypted_reasoning() {
    use crate::domain::message::ThinkingBlock;
    for body in [
        "short".to_string(),
        "x".repeat(HISTORY_PAGE_JSON_BUDGET * 2),
    ] {
        let mut message = Message::assistant(body, vec![]);
        message.thinking_blocks = vec![
            ThinkingBlock::Normal {
                thinking: "shown reasoning".into(),
                signature: String::new(),
            },
            ThinkingBlock::EncryptedReasoning {
                origin: "gpt-6-sol".into(),
                leads_to: None,
                item: r#"{"encrypted_content":"SECRET-BLOB"}"#.into(),
            },
        ];
        let json = super::message_to_json_for_history_page(&message).to_string();
        assert!(json.contains("shown reasoning"), "{json}");
        assert!(!json.contains("SECRET-BLOB"), "{json}");
        assert!(!json.contains("hidden"), "{json}");
    }
}

/// #2162 review: a message whose only thinking is encrypted carries no
/// `thinking` field at all, full or summarised.
#[test]
fn encrypted_only_thinking_adds_no_thinking_field() {
    use crate::domain::message::ThinkingBlock;
    for body in [
        "short".to_string(),
        "x".repeat(HISTORY_PAGE_JSON_BUDGET * 2),
    ] {
        let mut message = Message::assistant(body, vec![]);
        message.thinking_blocks = vec![ThinkingBlock::EncryptedReasoning {
            origin: "o".into(),
            leads_to: None,
            item: "{}".into(),
        }];
        let json = super::message_to_json_for_history_page(&message);
        assert!(json.get("thinking").is_none(), "{json}");
    }
}

/// #2226: each message carries its turn's origin (none when never stamped),
/// and every page names the transcript's report with a preview, even when
/// the report lies on an older page.
#[test]
fn messages_carry_their_turn_origin_and_the_page_names_the_report() {
    use crate::domain::turn_origin::{TurnOrigin, instruction, progress_nudge};
    let mut answer = Message::assistant("REPORT", vec![]);
    answer.turn_origin = TurnOrigin::Instruction;
    answer.ordinal = Some(2);
    let mut status = Message::assistant("status", vec![]);
    status.turn_origin = TurnOrigin::ProgressNudge;
    let messages = vec![
        instruction("task".into()),
        answer.clone(),
        progress_nudge("continue".into()),
        status,
        Message::assistant("unstamped", vec![]),
    ];
    let whole = messages_page_json(&messages, 5, None);
    let marks: Vec<Option<&str>> = page_messages(&whole)
        .iter()
        .map(|m| m.get("turnOrigin").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(
        marks,
        [
            Some("instruction"),
            Some("instruction"),
            Some("progressNudge"),
            Some("progressNudge"),
            None
        ]
    );
    let newest = messages_page_json(&messages, 1, None);
    assert_eq!(page_messages(&newest)[0]["content"], "unstamped");
    let report = &newest["report"];
    assert_eq!(report["id"], answer.id().to_string());
    assert_eq!(report["ordinal"], 2);
    assert_eq!(report["turnOrigin"], "instruction");
    assert_eq!(report["contentLength"], 6);
    let none = messages_page_json(&messages[..1], 1, None);
    assert!(none["report"].is_null());
}

/// #2226: the report a page names is small whatever the report's size, and
/// the page keeps room for it within its budget.
#[test]
fn the_named_report_is_small_and_fits_the_page_budget() {
    let long = "r".repeat(HISTORY_PAGE_JSON_BUDGET * 2);
    let mut messages = vec![Message::assistant(long, vec![])];
    messages.push(Message::assistant(
        "x".repeat(HISTORY_PAGE_JSON_BUDGET),
        vec![],
    ));
    let page = messages_page_json(&messages, 2, None);
    let hint = page["report"].to_string();
    assert!(hint.len() < super::REPORT_HINT_RESERVE_BYTES, "{hint}");
    assert_eq!(page["report"]["contentLength"], HISTORY_PAGE_JSON_BUDGET);
    assert!(serde_json::to_vec(&page).unwrap().len() <= HISTORY_PAGE_JSON_BUDGET);
}

/// #2246 review finding 3: a report never stamped (saved before #2226)
/// carries no `turnOrigin` at all, like a message never stamped, never a
/// `null` one.
#[test]
fn an_unmarked_report_names_no_turn_origin() {
    let messages = vec![Message::user("task"), Message::assistant("LEGACY", vec![])];
    let page = messages_page_json(&messages, 2, None);
    let report = page["report"].as_object().expect("the page names a report");
    assert!(!report.contains_key("turnOrigin"), "{page}");
    assert_eq!(page["report"]["contentLength"], 6);
    let reply = &page_messages(&page)[1];
    assert!(reply.get("turnOrigin").is_none(), "{page}");
}

/// #2404 (W2 review L4): every page message names its user kind, so the
/// TUI and a supervisor render a watermark cut's archive stub as a stub,
/// not as a prompt; an unmarked message names none, like `turnOrigin`.
#[test]
fn messages_carry_their_user_kind() {
    let messages = vec![
        crate::domain::turn_origin::prompt("the brief".into()),
        crate::domain::conversation::watermark_cut::archive_stub(7, Some("archive:2")),
        Message::user("feedback"),
        Message::assistant("answer", vec![]),
    ];
    let page = messages_page_json(&messages, 4, None);
    let kinds: Vec<Option<&str>> = page_messages(&page)
        .iter()
        .map(|m| m.get("userKind").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(kinds, [Some("prompt"), Some("archiveStub"), None, None]);
    let unmarked = &page_messages(&page)[2];
    assert!(unmarked.get("userKind").is_none(), "{page}");
}
