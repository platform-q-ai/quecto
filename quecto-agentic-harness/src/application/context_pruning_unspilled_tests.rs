//! PR #1048 follow-up: unspilled conversation content (`spill_id == None`,
//! the residue of a spill-append failure or a missing store at creation)
//! must never be stubbed into an unresolvable `recall("unknown")` — the
//! count trigger skips it, and the ladder lets it fall through to the second
//! rung's plain drop, as the pre-#1046 ceiling did. Split from
//! `context_pruning_message_tests.rs` for the 750-line source cap.

use super::messages::*;
use crate::domain::message::{Message, Role};

/// An old spilled conversation message (as production stamps after #1046 AC1).
fn spilled_msg(role: Role, turn: u32, i: u32) -> Message {
    let content = format!("conversation message {i} {}", "padding ".repeat(20));
    let mut m = match role {
        Role::Assistant => Message::assistant(&content, vec![]),
        _ => Message::user(&content),
    };
    m.turn = Some(turn);
    m.spill_id = Some(format!("turn{turn}:msg:{}", role.as_str()));
    m
}

/// An old conversation message whose creation-time spill FAILED: no spill_id.
fn unspilled_msg(turn: u32) -> Message {
    let mut m = Message::user(format!("unspilled old question {}", "padding ".repeat(20)));
    m.turn = Some(turn);
    m
}

#[test]
fn count_trigger_never_stubs_unspilled_messages() {
    let mut messages = vec![unspilled_msg(1)];
    for i in 2..=4u32 {
        messages.push(spilled_msg(Role::Assistant, i, i));
    }
    messages.push(Message::user("current question"));

    let collapsed = collapse_conversation_messages_over_limit(&mut messages, 0, 0);

    assert!(
        collapsed >= 1,
        "positive control: spilled old messages must still collapse"
    );
    assert!(
        !messages[0].is_collapsed && messages[0].content.starts_with("unspilled old question"),
        "a message that never reached the spill store must not be stubbed \
         (its recall() would be unresolvable); got: {}",
        messages[0].content
    );
    assert!(
        !messages
            .iter()
            .any(|m| m.content.contains("recall(\"unknown\")")),
        "no message may ever be stubbed to recall(\"unknown\")"
    );
}

#[test]
fn ladder_drops_unspilled_messages_plainly_instead_of_stubbing() {
    let mut messages = vec![unspilled_msg(1)];
    for i in 2..=4u32 {
        messages.push(spilled_msg(Role::Assistant, i, i));
    }
    messages.push(Message::user("current question"));

    // Budget of 5 forces full demotion: everything droppable goes.
    let outcome = enforce_context_ceiling_ladder(&mut messages, 5, 0);

    assert!(
        outcome.dropped >= 1,
        "positive control: the ladder must reach the second rung"
    );
    assert!(
        !messages
            .iter()
            .any(|m| m.content.contains("recall(\"unknown\")")),
        "an unspilled message must never be stubbed to recall(\"unknown\") — \
         it is dropped plainly (pre-#1046 ceiling behaviour) or kept intact"
    );
    assert!(
        !messages
            .iter()
            .any(|m| m.content.starts_with("unspilled old question") && m.is_collapsed),
        "the unspilled message must not be marked collapsed"
    );
}
