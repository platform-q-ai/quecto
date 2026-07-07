// #1046: conversation-message lifecycle — creation-time spilling, count-based
// collapse to recall stubs, and the demotion-ladder ceiling (stub → drop).
//
// Conversation (assistant/user) messages get the same lifecycle tool outputs
// already have: written to the spill store at creation (single writer:
// [`spill_conversation_message`]), proactively collapsed to inline `recall()`
// stubs once `context_collapse_after_messages` live messages accumulate, and
// demoted down the ladder (full → stub → removed) under token-budget pressure.
//
// Depends on: domain::message, domain::session (ContextSpillStore).
// Never imports infrastructure.

use super::{
    COLLAPSE_DISABLED, collapse_message, drop_until_under_budget, estimate_message_tokens,
    estimate_tokens, estimate_total_tokens, truncate_utf8_safe,
};
use crate::domain::message::{Message, Role};
use crate::domain::session::{ContextSpillStore, SpillEntry};

/// Outcome of one demotion-ladder ceiling pass (#1046 AC6, #1044 AC1).
#[derive(Debug, Clone, Default)]
pub struct CeilingLadderOutcome {
    /// Full conversation messages demoted to recall stubs (first rung).
    pub collapsed_to_stubs: usize,
    /// Stubs removed entirely, manifest-only (second rung).
    pub dropped: usize,
    /// True when the budget is still exceeded after full demotion — the
    /// pinned/exempt set alone is over budget (#1044).
    pub over_budget: bool,
}

/// Format the one-liner stub for a collapsed conversation message, e.g.
/// `[assistant: "<preview>" (840 tokens) — recall("turn12:msg:assistant")]`.
pub fn message_collapse_stub(role: &str, preview: &str, tokens: usize, spill_id: &str) -> String {
    // Flatten newlines so the stub stays a one-liner whatever the content.
    let preview = truncate_utf8_safe(preview, 60).replace(['\n', '\r'], " ");
    format!("[{role}: \"{preview}\" ({tokens} tokens) — recall(\"{spill_id}\")]")
}

/// Reduce a conversation collapse stub to its annotation by stripping the
/// trailing `— recall("…")` clause, e.g.
/// `[assistant: "<preview>" (840 tokens) — recall("id")]` →
/// `[assistant: "<preview>" (840 tokens)]`. Used by rewind: the spill store
/// is wiped, so retained stubs must not keep dangling recall pointers (the
/// same no-dangling-recall invariant tool stubs already honour).
pub fn message_stub_without_recall(stub: &str) -> String {
    match stub.find(" — recall(") {
        Some(pos) => format!("{}]", &stub[..pos]),
        None => stub.to_string(),
    }
}

fn is_conversation(msg: &Message) -> bool {
    matches!(msg.role, Role::User | Role::Assistant)
}

/// Replace a conversation message's content with its compact recall stub,
/// releasing the (already spilled) full content and any attachments. The
/// assistant's `tool_calls` are kept so matching tool-result messages never
/// become orphaned in the provider payload. Callers must pass the message's
/// own `spill_id` — unspilled content (`spill_id == None`, e.g. after a
/// spill-append failure) must never be stubbed, because its `recall()` would
/// be unresolvable; the collapse/ladder call sites skip such messages.
fn collapse_conversation_message(msg: &mut Message, spill_id: &str) {
    let tokens = estimate_tokens(&msg.content);
    msg.content = message_collapse_stub(msg.role.as_str(), &msg.content, tokens, spill_id);
    msg.invalidate_token_cache();
    msg.is_collapsed = true;
    msg.image_blocks.clear();
    msg.user_image_blocks.clear();
    msg.thinking_blocks.clear();
}

/// Per-message exemption flags for the count trigger and the demotion ladder
/// (#1046 AC3): pinned messages (system prompt, manifest), system messages,
/// the in-flight user prompt (last turn-less user message), turn-less
/// messages of the current prompt, and messages within the
/// `pin_recent_turns` most recent distinct turns.
///
/// Turn numbering restarts on every prompt, so the pinned tail is computed
/// from the current prompt's region (everything from the in-flight prompt
/// onward). With `tail_fallback` (the ladder), when the current prompt has
/// produced no turns yet the tail falls back to the previous prompt's turns,
/// so `pin_recent_turns` keeps protecting the most recent completed turns
/// between prompts (#1045). NOTE: this fallback is a deliberate behaviour
/// addition beyond #1044/#1045/#1046's literal ACs — the replaced
/// `enforce_context_ceiling_spilling` could drop the previous prompt's tail
/// when the current prompt had no turns yet; the ladder preserves the spirit
/// of tail-pinning between prompts instead (pinned by
/// `ceiling_ladder_tail_fallback_protects_previous_prompt_turns`).
/// The count trigger does not use the fallback: it
/// already keeps the most recent N messages in full by construction, and its
/// whole point is ageing out earlier prompts' prose.
fn exempt_flags(messages: &[Message], pin_recent_turns: u32, tail_fallback: bool) -> Vec<bool> {
    let region_start = messages
        .iter()
        .rposition(|m| m.role == Role::User && m.turn.is_none())
        .unwrap_or(0);
    // The turn-bearing region: the current prompt's region, or — when it has
    // no turns yet — the previous prompt's region.
    let mut tail_start = region_start;
    if tail_fallback && messages[region_start..].iter().all(|m| m.turn.is_none()) {
        tail_start = messages[..region_start]
            .iter()
            .rposition(|m| m.role == Role::User && m.turn.is_none())
            .unwrap_or(0);
    }
    let mut recent_turns: Vec<u32> = messages[tail_start..]
        .iter()
        .filter_map(|m| m.turn)
        .collect();
    recent_turns.sort_unstable();
    recent_turns.dedup();
    let keep_from = recent_turns.len().saturating_sub(pin_recent_turns as usize);
    let pinned_turns = &recent_turns[keep_from..];

    messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            m.is_pinned
                || m.role == Role::System
                || (i >= region_start && m.turn.is_none())
                || (i >= tail_start && m.turn.is_some_and(|t| pinned_turns.contains(&t)))
        })
        .collect()
}

/// Collapse the oldest live conversation (assistant + user, one combined
/// count) messages to recall stubs once their number exceeds `max_messages`
/// (#1046 AC2). Exempt from the count and never collapsed: system prompt,
/// manifest, the in-flight user prompt, and messages within the
/// `pin_recent_turns` tail. Tool results are excluded — the tool dial
/// (`context_collapse_after_tool_calls`) is independent.
/// `max_messages == COLLAPSE_DISABLED` disables. Returns collapsed count.
pub fn collapse_conversation_messages_over_limit(
    messages: &mut [Message],
    max_messages: u32,
    pin_recent_turns: u32,
) -> usize {
    if max_messages == COLLAPSE_DISABLED {
        return 0;
    }
    let exempt = exempt_flags(messages, pin_recent_turns, false);
    let live: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|&(i, m)| {
            // spill_id == None means the content never reached the spill
            // store (append failure / missing store): stubbing it would mint
            // an unresolvable recall(), so it is skipped entirely.
            !exempt[i] && is_conversation(m) && !m.is_collapsed && m.spill_id.is_some()
        })
        .map(|(i, _)| i)
        .collect();
    let to_collapse = live.len().saturating_sub(max_messages as usize);
    for &i in &live[..to_collapse] {
        // `live` filtered to spill_id.is_some(); skip defensively otherwise.
        let Some(spill_id) = messages[i].spill_id.clone() else {
            continue;
        };
        collapse_conversation_message(&mut messages[i], &spill_id);
    }
    to_collapse
}

/// Enforce the context ceiling by demoting down the ladder (#1046 AC6):
/// first collapse not-yet-collapsed messages to recall stubs (oldest first —
/// cheap, keeps locality), and only if still over budget remove stubs
/// entirely (manifest-only; content is already on disk from creation-time
/// spilling). Pinned/exempt messages are never demoted at any rung; when
/// they alone exceed the budget the outcome reports `over_budget` so the
/// caller can warn and audit (#1044).
pub fn enforce_context_ceiling_ladder(
    messages: &mut Vec<Message>,
    max_tokens: usize,
    pin_recent_turns: u32,
) -> CeilingLadderOutcome {
    let mut outcome = CeilingLadderOutcome::default();
    let mut total = estimate_total_tokens(messages);
    if total <= max_tokens {
        return outcome;
    }
    let exempt = exempt_flags(messages, pin_recent_turns, true);

    // First rung: demote full messages to stubs, oldest first. A message
    // whose stub would be no cheaper than its content (tiny messages) is
    // skipped — it goes straight to the second rung instead.
    for (i, msg) in messages.iter_mut().enumerate() {
        if total <= max_tokens {
            break;
        }
        if exempt[i] || msg.is_collapsed {
            continue;
        }
        // Unspilled conversation content (spill_id == None: ephemeral spill
        // failure or a missing store at creation) is never stubbed — its
        // recall() would be unresolvable. It falls through to the second
        // rung's plain drop, as the pre-#1046 ceiling did.
        if is_conversation(msg) && msg.spill_id.is_none() {
            continue;
        }
        let before = estimate_message_tokens(msg);
        let stub_tokens = estimate_tokens(&message_collapse_stub(
            msg.role.as_str(),
            &msg.content,
            before,
            msg.spill_id.as_deref().unwrap_or("unknown"),
        ));
        if stub_tokens >= before {
            continue;
        }
        match msg.role {
            Role::Tool => collapse_message(msg),
            Role::User | Role::Assistant => {
                // Guarded above: conversation messages here have a spill_id.
                let Some(spill_id) = msg.spill_id.clone() else {
                    continue;
                };
                collapse_conversation_message(msg, &spill_id);
            }
            Role::System => continue,
        }
        outcome.collapsed_to_stubs += 1;
        total = total.saturating_sub(before) + estimate_message_tokens(msg);
    }

    // Second rung: remove demoted messages entirely, oldest first (content
    // stays recallable via the spill store and manifest).
    if total > max_tokens {
        let droppable: Vec<usize> = messages
            .iter()
            .enumerate()
            .filter(|&(i, _)| !exempt[i])
            .map(|(i, _)| i)
            .collect();
        outcome.dropped = drop_until_under_budget(messages, max_tokens, &droppable).len();
    }

    outcome.over_budget = estimate_total_tokens(messages) > max_tokens;
    outcome
}

/// Spill a conversation (assistant/user) message to the store at creation
/// time (#1046 AC1) under `turn{N}:msg:{role}` — the single spill writer for
/// conversation content. Turn numbering restarts each prompt while the store
/// persists for the session, so the base id is de-duplicated against the
/// store index with a `:{n}` suffix (highest existing suffix + 1, computed in
/// one pass over the index). The message's `spill_id` is stamped so later
/// collapse/demotion can reference it. Ephemeral sessions (empty key)
/// deliberately persist too, matching tool-output spilling: collapse and the
/// demotion ladder can fire within a single `--no-session` run, and their
/// `recall()` stubs must stay resolvable, so entries are written under the
/// sanitized empty-key store path (PR #1048; see the NOTE in
/// `agent_loop_spill.rs`). Returns true when an entry was written (the
/// manifest needs a refresh).
pub async fn spill_conversation_message(
    msg: &mut Message,
    store: &dyn ContextSpillStore,
    session_key: &str,
) -> bool {
    if !is_conversation(msg) || msg.is_collapsed || msg.content.is_empty() {
        return false;
    }
    let role = msg.role.as_str();
    let base = format!("turn{}:msg:{role}", msg.turn.unwrap_or(0));
    let existing = store.list_entries(session_key).await.unwrap_or_default();
    // One linear pass: the next free suffix is (highest taken suffix) + 1,
    // where the bare base id counts as suffix 1. Avoids the quadratic
    // probe-and-rescan a `while any(id taken)` loop would cost per prompt.
    let mut max_n = 0usize;
    for e in existing.iter() {
        if e.id == base {
            max_n = max_n.max(1);
        } else if let Some(n) =
            e.id.strip_prefix(&base)
                .and_then(|rest| rest.strip_prefix(':'))
                .and_then(|n| n.parse::<usize>().ok())
        {
            max_n = max_n.max(n);
        }
    }
    let id = if max_n == 0 {
        base
    } else {
        format!("{base}:{}", max_n + 1)
    };
    // Move (not clone) the content into the SpillEntry for the borrowing
    // append, then move it back — avoids copying large message bodies on the
    // per-turn hot path (same pattern as the tool-output spill writer).
    let content = std::mem::take(&mut msg.content);
    let entry = SpillEntry {
        id: id.clone(),
        tool: role.to_string(),
        input_preview: truncate_utf8_safe(&content, 100).into_owned(),
        tokens: estimate_tokens(&content),
        content,
    };
    let result = store.append(session_key, &entry).await;
    // Restore content back into the message (entry is consumed here).
    msg.content = entry.content;
    match result {
        Ok(()) => {
            msg.spill_id = Some(id);
            true
        }
        Err(e) => {
            tracing::warn!(
                target: "context_prune",
                error = %e,
                "failed to spill conversation message"
            );
            false
        }
    }
}
