//! Red characterization of Rewind a conversation (#1865, D6 #1975): the
//! no-effect refusals, the exact effect order of an admitted rewind, the
//! best-effort retention clear, and the non-atomic save failure.
use super::super::conversation_rewrite_rig::{
    RewriteOptions, RewriteRig, build_rewrite_rig, contents,
};
use super::{reset_ledger_to, rewind_to_message_index};
use crate::application::sessions::dto::{RewindConversationError, RewindRequest};
use crate::domain::conversation_edit::RewindTarget;
use crate::domain::ids::MessageId;
use crate::domain::message::Message;

const PAGE: usize = 64;

fn conversation() -> Vec<Message> {
    vec![
        Message::system("Be helpful."),
        Message::user("first"),
        Message::assistant("answer", vec![]),
        Message::user("second"),
        Message::assistant("later", vec![]),
    ]
}

fn by_id(message: &Message) -> RewindRequest {
    RewindRequest {
        target: RewindTarget::MessageId(MessageId::from(message.id().to_string())),
        legacy_index_window: PAGE,
    }
}

fn by_index(index: usize) -> RewindRequest {
    RewindRequest {
        target: RewindTarget::LegacyIndex(index),
        legacy_index_window: PAGE,
    }
}

/// Assert a refused rewind touched nothing: no mutation, no ledger, no
/// accounting, no retention, no store.
async fn assert_no_effect(rig: &RewriteRig, request: RewindRequest, expected: &str) {
    let mut messages = conversation();
    rig.record(&messages[1..]);
    let before = messages.clone();
    rig.set_watermark(5);
    let mut accounting = rig.accounting();
    let err = rig
        .rewind
        .execute(&mut messages, &mut accounting, &request)
        .await
        .expect_err(expected);
    assert_eq!(err.to_string(), expected);
    assert!(err.ledger().is_none());
    assert_eq!(contents(&messages), contents(&before));
    assert!(before[1..].iter().all(|m| rig.resolves(m)));
    assert_eq!(rig.watermark(), 5);
    assert!(accounting.resets.is_empty());
    assert!(rig.journal().is_empty());
}

#[tokio::test]
async fn a_stale_or_unknown_stable_id_is_refused_without_effect() {
    let rig = build_rewrite_rig(RewriteOptions::default());
    let stranger = Message::user("never in this conversation");
    assert_no_effect(&rig, by_id(&stranger), "rewind target not found").await;
}

#[tokio::test]
async fn a_non_user_target_is_refused_without_effect() {
    let rig = build_rewrite_rig(RewriteOptions::default());
    assert_no_effect(&rig, by_index(2), "invalid rewind target").await;
    assert_no_effect(&rig, by_index(99), "invalid rewind target").await;
}

#[tokio::test]
async fn a_non_user_stable_id_is_refused_without_effect() {
    let rig = build_rewrite_rig(RewriteOptions::default());
    let mut messages = conversation();
    rig.record(&messages[1..]);
    let assistant = messages[2].clone();
    let mut accounting = rig.accounting();
    let err = rig
        .rewind
        .execute(&mut messages, &mut accounting, &by_id(&assistant))
        .await
        .expect_err("an assistant message is no boundary");
    assert!(matches!(err, RewindConversationError::InvalidTarget));
    assert_eq!(messages.len(), 5);
    assert!(rig.journal().is_empty());
}

#[tokio::test]
async fn a_legacy_index_beyond_one_page_is_refused_without_effect() {
    let rig = build_rewrite_rig(RewriteOptions::default());
    let mut messages: Vec<Message> = (0..=PAGE).map(|i| Message::user(format!("m{i}"))).collect();
    let mut accounting = rig.accounting();
    let err = rig
        .rewind
        .execute(
            &mut messages,
            &mut accounting,
            &RewindRequest {
                target: RewindTarget::LegacyIndex(2),
                legacy_index_window: PAGE,
            },
        )
        .await
        .expect_err("ambiguous");
    assert_eq!(
        err.to_string(),
        "messageIndex is ambiguous beyond one history page; rewind requires messageId"
    );
    assert_eq!(messages.len(), PAGE + 1);
    assert!(rig.journal().is_empty());
    // The stable id of the same message still rewinds.
    let target = messages[2].clone();
    rig.rewind
        .execute(&mut messages, &mut accounting, &by_id(&target))
        .await
        .expect("the id is never ambiguous");
    assert_eq!(messages.len(), 2);
}

#[tokio::test]
async fn an_admitted_rewind_truncates_resets_clears_retention_then_saves_in_that_order() {
    let rig = build_rewrite_rig(RewriteOptions {
        prompt: "Be helpful.",
        ..RewriteOptions::default()
    });
    let mut messages = conversation();
    rig.record(&messages[1..]);
    let (kept, target, dropped) = (
        messages[2].clone(),
        messages[3].clone(),
        messages[4].clone(),
    );
    rig.set_watermark(5);
    let epoch_before = rig.state.try_read().unwrap().conversation().epoch();
    let mut accounting = rig.accounting();

    let rewound = rig
        .rewind
        .execute(&mut messages, &mut accounting, &by_id(&target))
        .await
        .expect("rewind succeeds");

    assert_eq!(rewound.message_index, 3);
    // The selected user message and everything after it are gone.
    assert_eq!(contents(&messages), ["Be helpful.", "first", "answer"]);
    // Ledger reset to the survivors: a rewound-away ref stops resolving,
    // a survivor still does, and the epoch moved.
    assert_eq!(rewound.ledger.epoch, epoch_before + 1);
    assert!(rewound.ledger.changed);
    assert!(rig.resolves(&kept));
    assert!(!rig.resolves(&target));
    assert!(!rig.resolves(&dropped));
    // Watermark reset, then advanced by the save of the two written
    // messages (the prompt is stripped).
    assert_eq!(rig.watermark(), 2);
    assert_eq!(accounting.resets, [2]);
    assert_eq!(rig.cleared_namespaces(), ["cli:rewrite"]);
    assert_eq!(
        rig.journal(),
        [
            "accounting.reset(2)",
            "retention.clear",
            "store.save_clean_delta"
        ],
        "truncate → reset accounting → clear retention → save"
    );
    assert_eq!(
        rig.saved().iter().map(|s| contents(s)).collect::<Vec<_>>(),
        [vec!["first".to_string(), "answer".to_string()]]
    );
}

#[tokio::test]
async fn a_legacy_index_within_one_page_rewinds_like_the_id() {
    let rig = build_rewrite_rig(RewriteOptions::default());
    let mut messages = conversation();
    let mut accounting = rig.accounting();
    let rewound = rig
        .rewind
        .execute(&mut messages, &mut accounting, &by_index(1))
        .await
        .unwrap();
    assert_eq!(rewound.message_index, 1);
    assert_eq!(contents(&messages), ["Be helpful."]);
}

#[tokio::test]
async fn a_retention_clear_failure_is_warning_only_and_the_save_still_runs() {
    let rig = build_rewrite_rig(RewriteOptions {
        retention: Some(true),
        ..RewriteOptions::default()
    });
    let mut messages = conversation();
    let mut accounting = rig.accounting();
    rig.rewind
        .execute(&mut messages, &mut accounting, &by_index(3))
        .await
        .expect("a retention failure does not fail the rewind");
    assert_eq!(
        rig.journal(),
        [
            "accounting.reset(3)",
            "retention.clear",
            "store.save_clean_delta"
        ]
    );
    assert_eq!(rig.saved().len(), 1);
}

#[tokio::test]
async fn a_save_failure_returns_the_persistence_error_with_the_rewind_left_visible() {
    let rig = build_rewrite_rig(RewriteOptions {
        save_fails: true,
        ..RewriteOptions::default()
    });
    let mut messages = conversation();
    rig.record(&messages[1..]);
    let dropped = messages[4].clone();
    rig.set_watermark(5);
    let mut accounting = rig.accounting();

    let err = rig
        .rewind
        .execute(&mut messages, &mut accounting, &by_index(3))
        .await
        .expect_err("the store failed");

    assert_eq!(
        err.to_string(),
        "failed to save rewound session: session error: disk full"
    );
    assert!(err.ledger().is_some_and(|l| l.changed));
    assert_eq!(contents(&messages), ["Be helpful.", "first", "answer"]);
    assert!(!rig.resolves(&dropped));
    assert_eq!(rig.watermark(), 0);
    assert_eq!(accounting.resets, [3]);
    assert_eq!(rig.cleared_namespaces(), ["cli:rewrite"]);
    assert!(rig.saved().is_empty());
}

#[tokio::test]
async fn an_ephemeral_session_rewinds_resets_the_watermark_and_saves_nothing() {
    let rig = build_rewrite_rig(RewriteOptions {
        ephemeral: true,
        ..RewriteOptions::default()
    });
    let mut messages = conversation();
    rig.set_watermark(5);
    let mut accounting = rig.accounting();
    rig.rewind
        .execute(&mut messages, &mut accounting, &by_index(3))
        .await
        .unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(rig.journal(), ["accounting.reset(3)", "retention.clear"]);
    assert!(rig.saved().is_empty());
    assert_eq!(rig.watermark(), 0);
}

#[test]
fn rewinding_strips_retention_residue_from_the_survivors() {
    let mut manifest = Message::system("[Session memory: 1 spilled entry]");
    manifest.is_manifest = true;
    let mut collapsed = Message::tool("call-1", "[bash: output — recall(\"turn1:bash:0\")]");
    collapsed.is_collapsed = true;
    collapsed.spill_id = Some("turn1:bash:0".into());
    let mut messages = vec![
        Message::system("Be helpful."),
        manifest,
        Message::user("first"),
        collapsed,
        Message::user("second"),
    ];
    assert!(rewind_to_message_index(&mut messages, 4));
    assert!(!messages.iter().any(|m| m.is_manifest));
    assert!(!messages.iter().any(|m| m.is_collapsed));
    assert!(!messages.iter().any(|m| m.spill_id.is_some()));
    assert!(!messages.iter().any(|m| m.content.contains("recall(")));
    assert_eq!(
        messages[2].content, "",
        "a collapsed tool result is blanked"
    );
}

#[test]
fn rewinding_keeps_collapsed_conversation_messages_as_non_empty_turns() {
    // #1046: `is_collapsed` no longer implies a tool stub. A collapsed
    // user/assistant message survives as a NON-EMPTY provider turn (its
    // stub minus the dangling recall clause) — some providers reject empty
    // text blocks.
    use crate::application::context_pruning::messages::message_collapse_stub;
    let mut collapsed_user =
        Message::user("[user: \"old question\" (120 tokens) — recall(\"turn1:msg:user\")]");
    collapsed_user.is_collapsed = true;
    collapsed_user.spill_id = Some("turn1:msg:user".into());
    let mut collapsed_assistant = Message::assistant(
        message_collapse_stub(
            "assistant",
            "I analysed the logs",
            840,
            "turn2:msg:assistant",
        ),
        vec![],
    );
    collapsed_assistant.is_collapsed = true;
    collapsed_assistant.spill_id = Some("turn2:msg:assistant".into());
    let mut messages = vec![
        Message::system("system prompt"),
        collapsed_user,
        collapsed_assistant,
        Message::user("rewind target"),
        Message::assistant("later answer", vec![]),
    ];
    assert!(rewind_to_message_index(&mut messages, 3));
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[1].content, "[user: \"old question\" (120 tokens)]");
    assert!(
        messages[2]
            .content
            .starts_with("[assistant: \"I analysed the logs\"")
            && !messages[2].content.contains("recall("),
        "{}",
        messages[2].content
    );
    for m in &messages {
        assert!(!m.is_collapsed);
        assert!(m.spill_id.is_none());
    }
    assert!(
        !rewind_to_message_index(&mut messages, 2),
        "not a user boundary"
    );
    assert!(!rewind_to_message_index(&mut messages, 9), "out of range");
    assert_eq!(messages.len(), 3);
}

#[test]
fn the_ledger_reset_bumps_the_epoch_once_without_double_counting_republished_messages() {
    let mut state = crate::application::sessions::active_session::ActiveSessionState::new(
        crate::domain::session_identity::SessionIdentity::ephemeral(),
    );
    state.publish(&[Message::user("old")]);
    let epoch = state.conversation().epoch();
    let rev = state.conversation().rev();
    let advance = reset_ledger_to(&mut state, &[Message::user("new")]);
    assert_eq!((advance.epoch, advance.rev), (epoch + 1, rev + 1));
    assert!(advance.changed);
    assert_eq!(state.conversation().epoch(), epoch + 1);
    assert_eq!(state.conversation().rev(), rev + 1);
}

#[test]
fn the_use_case_debug_names_itself_without_its_handles() {
    let rig = build_rewrite_rig(RewriteOptions::default());
    assert_eq!(format!("{:?}", rig.rewind), "RewindConversation { .. }");
}
