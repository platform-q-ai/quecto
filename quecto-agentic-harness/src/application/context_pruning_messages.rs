// #1046: conversation-message lifecycle — creation-time spilling and the
// emergency ladder's stubs (stub → drop, #2414: only when no watermark cut
// can bring a request under the ceiling).
//
// Conversation (assistant/user) messages are written to the spill store at
// creation (single writer: [`spill_conversation_message`]), so a ladder
// stub or drop stays recallable.
//
// Depends on: domain::conversation::value_objects::message and its stored images, application::sessions::use_cases (the narrow
// retention writer, D9 #1978). Never imports infrastructure, never reaches the retention store.

use super::{estimate_tokens, truncate_utf8_safe};
use crate::application::sessions::use_cases::RetainContext;
use crate::domain::conversation::services::turn_origin::latest_opener;
use crate::domain::conversation::value_objects::message::{Message, Role};
use crate::domain::conversation::value_objects::stored_images as images;
use crate::domain::sessions::entities::session::SpillEntry;

// #2213: the emergency ladder and its low-water mark.
#[path = "context_pruning_ceiling.rs"]
pub(super) mod ceiling;
pub use ceiling::{CeilingLadderOutcome, enforce_context_ceiling_ladder};

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
///
/// `rfind`, not `find`: the 60-char preview is arbitrary user text and can
/// itself contain ` — recall(` (e.g. a pasted stub); the real clause is the
/// one the formatter appends, which is always last.
pub fn message_stub_without_recall(stub: &str) -> String {
    match stub.rfind(" — recall(") {
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
/// be unresolvable; the ladder skips such messages.
fn collapse_conversation_message(msg: &mut Message, spill_id: &str) {
    let tokens = estimate_tokens(&msg.content);
    msg.content = message_collapse_stub(msg.role.as_str(), &msg.content, tokens, spill_id);
    msg.invalidate_token_cache();
    msg.is_collapsed = true;
    images::release_images(msg);
    msg.thinking_blocks.clear();
}

/// Per-message exemption flags for the emergency ladder (#1046 AC3):
/// pinned messages (system prompt, manifest), system messages, the
/// in-flight user prompt (last turn-less user message), turn-less messages
/// of the current prompt, and messages within the `pin_recent_turns` most
/// recent distinct turns.
///
/// Turn numbering restarts on every prompt, so the pinned tail is computed
/// from the current prompt's region (everything from the in-flight prompt
/// onward). When the current prompt has produced no turns yet the tail
/// falls back to the previous prompt's turns, so `pin_recent_turns` keeps
/// protecting the most recent completed turns between prompts (#1045;
/// pinned by `ceiling_ladder_tail_fallback_protects_previous_prompt_turns`).
fn exempt_flags(messages: &[Message], pin_recent_turns: u32) -> Vec<bool> {
    let region_start = latest_opener(messages).unwrap_or(0);
    // The turn-bearing region: the current prompt's region, or — when it has
    // no turns yet — the previous prompt's region.
    let mut tail_start = region_start;
    if messages[region_start..].iter().all(|m| m.turn.is_none()) {
        tail_start = latest_opener(&messages[..region_start]).unwrap_or(0);
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

/// Spill a conversation (assistant/user) message to the store at creation time (#1046 AC1) under
/// `turn{N}:msg:{role}` — the single spill writer for conversation content (an image-only prompt
/// too, previewed `[image]`, #2424). Turn numbering restarts each prompt while the store persists
/// for the session, so the base id is de-duplicated by the sessions capability with a `:{n}` suffix
/// (`RetainContext::retain_deduplicated`, D9 #1978: highest existing suffix + 1, one pass over the
/// index); the policy allocates the base id, sessions appends and issues the id it retained. The
/// message's `spill_id` is stamped with that id so a later archive or ladder stub can reference it.
/// Ephemeral sessions (empty key) deliberately persist too, matching tool-output spilling: a cut
/// and the ladder can fire within a single `--no-session` run, and their `recall()` stubs must stay
/// resolvable, so entries are written under the sanitized empty-key store path (PR #1048; see the
/// NOTE in `agent_loop_spill.rs`). The privacy counterpart lives at the interface layer: ephemeral
/// run paths scrub the empty-key spill file at run end. Returns true when an entry was written (the
/// manifest needs a refresh).
pub async fn spill_conversation_message(
    msg: &mut Message,
    retain: &RetainContext,
    session_key: &crate::domain::sessions::entities::session_identity::SessionIdentity,
) -> bool {
    if !is_conversation(msg) || msg.is_collapsed || !images::has_text_or_user_image(msg) {
        return false;
    }
    let role = msg.role.as_str();
    let base = format!("turn{}:msg:{role}", msg.turn.unwrap_or(0));
    // Move (not clone) the content into the SpillEntry for the borrowing
    // append, then move it back — avoids copying large message bodies on the
    // per-turn hot path (same pattern as the tool-output spill writer).
    let content = std::mem::take(&mut msg.content);
    let mut entry = SpillEntry {
        id: base,
        tool: role.to_string(),
        input_preview: match content.is_empty() {
            true => "[image]".to_string(),
            false => truncate_utf8_safe(&content, 100).into_owned(),
        },
        tokens: estimate_tokens(&content) + images::image_tokens(msg),
        content,
        images: images::MessageImageRefs::of(msg).into_all(),
    };
    let result = retain.retain_deduplicated(session_key, &mut entry).await;
    // Restore content back into the message (entry is consumed here).
    msg.content = entry.content;
    match result {
        Ok(retained) => {
            msg.spill_id = Some(retained.id);
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
