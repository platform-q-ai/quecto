use super::{ConversationLedger, LEDGER_MAX_ENTRIES};
use crate::domain::message::{Message, ThinkingBlock};

/// #1060 review 1a: the id-addressable ledger keeps a ref resolvable after the
/// live conversation drops or collapses the referenced message.
#[test]
fn ledger_resolves_refs_after_prune_and_collapse() {
    let a = Message::assistant("full answer A", vec![]);
    let b = Message::assistant("full answer B", vec![]);
    let (a_id, b_id) = (a.id().to_string(), b.id().to_string());

    let mut ledger = ConversationLedger::default();
    ledger.publish(&[a.clone(), b.clone()]);
    assert!(ledger.lookup(&a_id).is_some() && ledger.lookup(&b_id).is_some());

    // The ladder DROPS A from the live conversation (publish without it).
    ledger.publish(std::slice::from_ref(&b));
    assert!(
        ledger.lookup(&a_id).is_some(),
        "a dropped ref must still resolve via the ledger"
    );
    assert_eq!(ledger.live_messages().len(), 1);

    // The ladder COLLAPSES B in place (same id, stub content). publish must not
    // clobber the full copy already in the ledger.
    let mut b_stub = b.clone();
    b_stub.content = "recall(spilled)".to_string();
    ledger.publish(&[b_stub.clone()]);
    assert_eq!(
        ledger.lookup(&b_id).map(|m| m.content.as_str()),
        Some("full answer B"),
        "the ledger's full copy must win over a collapsed live stub"
    );
    assert_eq!(
        ledger.full_copy(&b_id).map(|m| m.content.as_str()),
        Some("full answer B")
    );

    // record_full overwrites with an authoritative full copy (un-demoted).
    let mut ledger2 = ConversationLedger::default();
    ledger2.publish(std::slice::from_ref(&b_stub));
    ledger2.record_full(std::slice::from_ref(&b));
    assert_eq!(
        ledger2.lookup(&b_id).map(|m| m.content.as_str()),
        Some("full answer B")
    );
}

/// #1060 review r4 finding 2: the ledger is byte-bounded and evicts oldest-first,
/// so a weeks-long session cannot grow it without limit. The oldest refs stop
/// resolving once the budget is exceeded; the most recent still resolve.
#[test]
fn ledger_is_byte_bounded_and_evicts_oldest() {
    // Each message ~6 MiB of content; the 16 MiB budget holds ~2. Recording 4
    // in order must evict the two oldest.
    let big = || Message::assistant("X".repeat(6 * 1024 * 1024), vec![]);
    let msgs: Vec<Message> = (0..4).map(|_| big()).collect();
    let ids: Vec<String> = msgs.iter().map(|m| m.id().to_string()).collect();

    let mut ledger = ConversationLedger::default();
    ledger.record_full(&msgs);

    assert!(
        ledger.lookup(&ids[0]).is_none() && ledger.lookup(&ids[1]).is_none(),
        "the oldest refs must be evicted once the ledger byte budget is exceeded"
    );
    assert!(
        ledger.lookup(&ids[3]).is_some(),
        "the most recent ref must still resolve"
    );
}

#[test]
fn ledger_counts_thinking_blocks_against_byte_budget() {
    let make_msg = || {
        let mut msg = Message::assistant("ok", vec![]);
        msg.thinking_blocks.push(ThinkingBlock::Normal {
            thinking: "r".repeat(6 * 1024 * 1024),
            signature: "sig".into(),
        });
        msg
    };
    let msgs: Vec<Message> = (0..4).map(|_| make_msg()).collect();
    let ids: Vec<String> = msgs.iter().map(|m| m.id().to_string()).collect();

    let mut ledger = ConversationLedger::default();
    ledger.record_full(&msgs);

    assert!(ledger.lookup(&ids[0]).is_none());
    assert!(ledger.lookup(ids.last().unwrap()).is_some());
}

/// #1060 review r4 finding 2 (follow-up): the ledger must ALSO cap entry
/// count, so a flood of tiny/empty/tool-metadata messages cannot grow it
/// without bound even though the byte budget is far from full.
#[test]
fn ledger_is_entry_bounded_for_tiny_messages() {
    let mut ledger = ConversationLedger::default();
    let msgs: Vec<Message> = (0..LEDGER_MAX_ENTRIES + 100)
        .map(|_| Message::assistant("", vec![]))
        .collect();
    let ids: Vec<String> = msgs.iter().map(|m| m.id().to_string()).collect();
    ledger.record_full(&msgs);

    assert!(
        ledger.lookup(&ids[0]).is_none(),
        "the oldest tiny messages must be evicted by the entry-count cap"
    );
    assert!(
        ledger.lookup(ids.last().unwrap()).is_some(),
        "the most recent message must still resolve"
    );
}

#[test]
fn publish_advances_rev_only_for_new_ids_and_clear_opens_a_new_epoch() {
    let mut ledger = ConversationLedger::default();
    let msg = Message::user("one");
    let first = ledger.publish(std::slice::from_ref(&msg));
    let second = ledger.publish(std::slice::from_ref(&msg));
    assert!(first.changed);
    assert!(!second.changed);
    assert_eq!(first.rev, second.rev);
    assert_eq!(ledger.rev(), 1);
    assert_eq!(ledger.frontier().count(), 1);

    let epoch = ledger.epoch();
    let generation = ledger.generation();
    let cleared = ledger.clear();
    assert!(cleared.changed);
    assert_eq!(cleared.epoch, epoch + 1);
    assert_eq!(cleared.rev, 1, "clear keeps the revision counter");
    assert_eq!(ledger.generation(), generation + 1);
    assert!(ledger.live_messages().is_empty());
    assert!(ledger.lookup(&msg.id().to_string()).is_none());
    assert_eq!(ledger.frontier().count(), 0);
}

#[test]
fn retained_messages_are_ledger_copies_first_then_unretained_live_entries() {
    let mut ledger = ConversationLedger::default();
    let full = Message::assistant("full", vec![]);
    let live_only = Message::user("live");
    ledger.record_full(std::slice::from_ref(&full));
    let mut stub = full.clone();
    stub.content = "stub".into();
    ledger.publish(&[stub, live_only.clone()]);
    let retained = ledger.retained_messages();
    assert_eq!(retained.len(), 2);
    assert_eq!(retained[0].content, "full");
    assert_eq!(retained[1].id(), live_only.id());
}

#[test]
fn set_spill_store_advances_the_generation_only_when_the_handle_changes() {
    use crate::application::sessions::ports::ContextSpillStore;
    use std::sync::Arc;
    let mut ledger = ConversationLedger::default();
    assert!(ledger.spill_store().is_none());
    let store: Arc<dyn ContextSpillStore> =
        Arc::new(crate::application::sessions::use_cases::recover_message::tests::NoopSpillStore);
    ledger.set_spill_store(Some(store.clone()));
    assert_eq!(ledger.generation(), 1);
    ledger.set_spill_store(Some(store.clone()));
    assert_eq!(ledger.generation(), 1, "the same handle is not a change");
    ledger.set_spill_store(None);
    assert_eq!(ledger.generation(), 2);
    ledger.set_spill_store(None);
    assert_eq!(ledger.generation(), 2);
    assert!(ledger.spill_store().is_none());
}
