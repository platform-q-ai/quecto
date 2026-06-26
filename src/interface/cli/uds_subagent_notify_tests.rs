/// Unit tests for #816 auto-await subagent completion notes (enqueue + idle
/// delivery). Compiled as a `mod` inside `uds.rs`, so `super` = `uds`, which
/// re-exports `AgentSession` via `uds_session`.
use super::*;

// ─── #816: auto-await subagent completion notes (enqueue + idle delivery) ─────

/// A successful enqueue buffers exactly one pending note carrying the child id
/// and the one-line summary, rendered as a `role:"system"` message — the operator
/// channel — so it surfaces only at the next idle drain, never mid-turn.
#[test]
fn test_enqueue_subagent_notification_buffers_system_note() {
    use crate::domain::message::Role;
    let mut session = AgentSession::new("m".to_string(), "k".to_string());
    let enqueued = session.enqueue_subagent_notification(
        "researcher".to_string(),
        1,
        "[subagent] Agent 'researcher' completed. Last output: all tests pass".to_string(),
    );
    assert!(enqueued, "first completion note should be enqueued");
    assert_eq!(
        session.state_snapshot(0, None, 0).pending_message_count,
        1,
        "note must be buffered for the next idle drain, not delivered immediately"
    );
    let drained = session.drain_pending();
    assert_eq!(drained.len(), 1);
    let msg = drained.into_iter().next().unwrap().into_message();
    assert_eq!(msg.role, Role::System, "note is an operator/system message");
    assert!(msg.content.contains("researcher"));
    assert!(msg.content.contains("all tests pass"));
}

/// The same completion (identical agent_id + sequence) must inject exactly once:
/// the passive broadcast path also records it, so re-enqueue is deduped.
#[test]
fn test_enqueue_subagent_notification_dedupes_same_sequence() {
    let mut session = AgentSession::new("m".to_string(), "k".to_string());
    assert!(session.enqueue_subagent_notification("a".to_string(), 5, "done".to_string()));
    assert!(
        !session.enqueue_subagent_notification("a".to_string(), 5, "done".to_string()),
        "re-enqueuing the same sequence must be deduped"
    );
    assert!(
        !session.enqueue_subagent_notification("a".to_string(), 3, "stale".to_string()),
        "an older sequence must be deduped"
    );
    assert_eq!(
        session.state_snapshot(0, None, 0).pending_message_count,
        1,
        "dedupe must leave exactly one pending note"
    );
}

/// Multiple still-pending completions for the SAME agent coalesce into one note
/// (latest wins) so a noisy child does not cost N extra LLM turns.
#[test]
fn test_enqueue_subagent_notification_coalesces_same_agent() {
    let mut session = AgentSession::new("m".to_string(), "k".to_string());
    assert!(session.enqueue_subagent_notification("a".to_string(), 1, "first".to_string()));
    assert!(session.enqueue_subagent_notification("a".to_string(), 2, "second".to_string()));
    let drained = session.drain_pending();
    assert_eq!(
        drained.len(),
        1,
        "two completions for the same agent coalesce into one pending note"
    );
    let content = drained.into_iter().next().unwrap().into_message().content;
    assert!(
        content.contains("second"),
        "coalesced note keeps the latest summary, got: {content}"
    );
}

/// Distinct children each get their own note, delivered once apiece.
#[test]
fn test_enqueue_subagent_notification_distinct_agents() {
    let mut session = AgentSession::new("m".to_string(), "k".to_string());
    assert!(session.enqueue_subagent_notification("a".to_string(), 1, "a done".to_string()));
    assert!(session.enqueue_subagent_notification("b".to_string(), 1, "b done".to_string()));
    assert_eq!(session.drain_pending().len(), 2);
}

/// A failure (errored/exited) note is enqueued just like a completion — silence
/// is not acceptable.
#[test]
fn test_enqueue_subagent_notification_carries_failure() {
    let mut session = AgentSession::new("m".to_string(), "k".to_string());
    assert!(session.enqueue_subagent_notification(
        "linter".to_string(),
        1,
        "[subagent] Agent 'linter' errored: rate limit exceeded".to_string(),
    ));
    let content = session
        .drain_pending()
        .into_iter()
        .next()
        .unwrap()
        .into_message()
        .content;
    assert!(content.contains("linter"));
    assert!(content.contains("rate limit exceeded"));
}

/// Idle-timing contract: a completion that arrives while the parent is mid-turn
/// must be BUFFERED, not injected into the running turn. `enqueue_*` only ever
/// appends to the pending queue — it never renders or runs the note — so the
/// note becomes a turn only when the dispatch loop reaches idle and calls
/// `drain_pending`. This test stands in for the live drain boundary: after an
/// "active turn" arrival the note is queued (count==1) and the conversation
/// history is untouched until the explicit idle drain consumes it.
#[test]
fn test_enqueue_subagent_notification_is_buffered_until_idle_drain() {
    let mut session = AgentSession::new("m".to_string(), "k".to_string());

    // Arrival "during" the parent's active turn: only buffered.
    assert!(session.enqueue_subagent_notification(
        "researcher".to_string(),
        1,
        "researcher complete".to_string(),
    ));
    assert_eq!(
        session.state_snapshot(0, None, 0).pending_message_count,
        1,
        "a mid-turn arrival must be held in the pending queue, not delivered"
    );

    // The idle boundary (drain) is the ONLY place the buffered note is consumed.
    let drained = session.drain_pending();
    assert_eq!(
        drained.len(),
        1,
        "the idle drain delivers the buffered note"
    );
    assert_eq!(
        session.state_snapshot(0, None, 0).pending_message_count,
        0,
        "after the idle drain the note is consumed exactly once"
    );
}

/// The completion summary surfaced to the parent is a single line (≤1 line) so it
/// costs at most one operator turn and reads cleanly — asserted on the source
/// `SubagentNotification::to_message` strings that feed the enqueue path.
#[test]
fn test_subagent_notification_summary_is_single_line() {
    use crate::infrastructure::tools::subagent_registry::SubagentNotification;
    let notes = [
        SubagentNotification::Completed {
            agent_id: "worker".to_string(),
            summary: "all tests pass".to_string(),
        }
        .to_message(),
        SubagentNotification::Errored {
            agent_id: "linter".to_string(),
            error: "rate limit exceeded".to_string(),
        }
        .to_message(),
        SubagentNotification::Exited {
            agent_id: "worker".to_string(),
        }
        .to_message(),
    ];
    for note in notes {
        assert!(
            !note.contains('\n'),
            "completion note must be a single line, got: {note:?}"
        );
    }
}
