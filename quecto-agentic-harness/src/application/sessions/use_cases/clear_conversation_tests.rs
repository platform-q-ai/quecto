//! Red characterization of Clear conversation history (#1864, D6 #1975):
//! the exact effect order, the best-effort retention clear, and the
//! non-atomic save failure.
use super::super::conversation_rewrite_rig::{RewriteOptions, build_rewrite_rig, contents};
use crate::application::sessions::dto::ClearConversationError;
use crate::domain::message::Message;

fn conversation(prompt: &str) -> Vec<Message> {
    let mut messages = Vec::new();
    if !prompt.is_empty() {
        messages.push(Message::system(prompt));
    }
    messages.push(Message::user("first"));
    messages.push(Message::assistant("answer", vec![]));
    messages.push(Message::user("second"));
    messages
}

#[tokio::test]
async fn an_admitted_clear_mutates_resets_clears_retention_then_saves_in_that_order() {
    let rig = build_rewrite_rig(RewriteOptions {
        prompt: "Be helpful.",
        ..RewriteOptions::default()
    });
    let mut messages = conversation("Be helpful.");
    rig.record(&messages[1..]);
    let dropped = messages[2].clone();
    rig.set_watermark(3);
    let epoch_before = rig.state.try_read().unwrap().conversation().epoch();
    let mut accounting = rig.accounting();

    let cleared = rig
        .clear
        .execute(&mut messages, &mut accounting)
        .await
        .expect("clear succeeds");

    // Conversation mutation: only the injected prompt survives.
    assert_eq!(contents(&messages), ["Be helpful."]);
    // Snapshot/ledger invalidation: a new epoch, nothing retained.
    assert_eq!(cleared.ledger.epoch, epoch_before + 1);
    assert!(cleared.ledger.changed);
    assert!(rig.live_contents().is_empty());
    assert!(
        !rig.resolves(&dropped),
        "a cleared-away ref stops resolving"
    );
    // Persisted watermark reset, then the save advanced it to the
    // written length (the prompt is stripped: nothing).
    assert_eq!(rig.watermark(), 0);
    // Message-count synchronization and usage/pending reset: the visible
    // count excludes the injected prompt.
    assert_eq!(accounting.resets, [0]);
    // Best-effort retention clear on the current namespace, then save.
    assert_eq!(rig.cleared_namespaces(), ["cli:rewrite"]);
    assert_eq!(
        rig.journal(),
        [
            "accounting.reset(0)",
            "retention.clear",
            "store.save_clean_delta"
        ],
        "mutate → reset accounting → clear retention → save"
    );
    assert_eq!(rig.saved().len(), 1);
    assert!(
        rig.saved()[0].is_empty(),
        "the prompt is stripped: nothing written"
    );
    // The injected prompt is back at the head after the save.
    assert_eq!(contents(&messages), ["Be helpful."]);
}

#[tokio::test]
async fn a_watermark_reset_forces_a_full_rewrite_of_the_shrunk_history() {
    let rig = build_rewrite_rig(RewriteOptions::default());
    let mut messages = conversation("");
    rig.set_watermark(3);
    let mut accounting = rig.accounting();
    rig.clear
        .execute(&mut messages, &mut accounting)
        .await
        .unwrap();
    assert_eq!(
        rig.journal().last().map(String::as_str),
        Some("store.save_clean_delta"),
        "a clean delta from a zero watermark rewrites the file whole"
    );
    assert!(messages.is_empty());
    assert_eq!(accounting.resets, [0]);
}

#[tokio::test]
async fn a_retention_clear_failure_is_warning_only_and_the_save_still_runs() {
    let rig = build_rewrite_rig(RewriteOptions {
        retention: Some(true),
        ..RewriteOptions::default()
    });
    let mut messages = conversation("");
    let mut accounting = rig.accounting();
    let cleared = rig
        .clear
        .execute(&mut messages, &mut accounting)
        .await
        .expect("a retention failure does not fail the clear");
    assert!(cleared.ledger.changed);
    assert_eq!(rig.cleared_namespaces(), ["cli:rewrite"]);
    assert_eq!(
        rig.journal(),
        [
            "accounting.reset(0)",
            "retention.clear",
            "store.save_clean_delta"
        ]
    );
    assert_eq!(rig.saved().len(), 1);
}

#[tokio::test]
async fn without_a_retention_store_the_clear_skips_straight_to_the_save() {
    let rig = build_rewrite_rig(RewriteOptions {
        retention: None,
        ..RewriteOptions::default()
    });
    let mut messages = conversation("");
    let mut accounting = rig.accounting();
    rig.clear
        .execute(&mut messages, &mut accounting)
        .await
        .unwrap();
    assert_eq!(
        rig.journal(),
        ["accounting.reset(0)", "store.save_clean_delta"]
    );
}

#[tokio::test]
async fn a_save_failure_returns_the_persistence_error_with_the_mutation_left_visible() {
    let rig = build_rewrite_rig(RewriteOptions {
        save_fails: true,
        ..RewriteOptions::default()
    });
    let mut messages = conversation("");
    rig.record(&messages);
    let dropped = messages[0].clone();
    rig.set_watermark(3);
    let mut accounting = rig.accounting();

    let err = rig
        .clear
        .execute(&mut messages, &mut accounting)
        .await
        .expect_err("the store failed");

    assert_eq!(
        err.to_string(),
        "failed to save cleared session: session error: disk full"
    );
    let ClearConversationError::Save { ledger, .. } = &err;
    assert!(ledger.changed, "the ledger position is still announced");
    // No rollback: everything already cleared stays cleared.
    assert!(messages.is_empty());
    assert!(rig.live_contents().is_empty());
    assert!(!rig.resolves(&dropped));
    assert_eq!(rig.watermark(), 0);
    assert_eq!(accounting.resets, [0]);
    assert_eq!(rig.cleared_namespaces(), ["cli:rewrite"]);
    assert_eq!(
        rig.journal(),
        [
            "accounting.reset(0)",
            "retention.clear",
            "store.save_clean_delta"
        ]
    );
    assert!(rig.saved().is_empty());
}

#[tokio::test]
async fn an_ephemeral_session_clears_everything_and_saves_nothing() {
    let rig = build_rewrite_rig(RewriteOptions {
        ephemeral: true,
        ..RewriteOptions::default()
    });
    let mut messages = conversation("");
    let mut accounting = rig.accounting();
    rig.clear
        .execute(&mut messages, &mut accounting)
        .await
        .unwrap();
    assert!(messages.is_empty());
    assert_eq!(rig.journal(), ["accounting.reset(0)", "retention.clear"]);
    assert!(rig.saved().is_empty());
}

#[test]
fn the_use_case_debug_names_itself_without_its_handles() {
    let rig = build_rewrite_rig(RewriteOptions::default());
    assert_eq!(format!("{:?}", rig.clear), "ClearConversation { .. }");
}
