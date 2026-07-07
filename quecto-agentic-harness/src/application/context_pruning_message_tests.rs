//! #1046: count-based conversation-message collapse, demotion-ladder ceiling,
//! and creation-time message spilling. Split from `context_pruning.rs` to
//! respect the 750-line source cap.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use super::messages::*;
use super::*;
use crate::domain::message::{Message, Role};
use crate::domain::session::SpillEntry;

/// Minimal in-memory spill store for the creation-time spill path.
#[derive(Debug, Default)]
struct MemStore {
    entries: Mutex<Vec<SpillEntry>>,
}

impl ContextSpillStore for MemStore {
    fn append(
        &self,
        _session_key: &str,
        entry: &SpillEntry,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::domain::error::DomainError>> + Send + '_>>
    {
        self.entries.lock().unwrap().push(entry.clone());
        Box::pin(async { Ok(()) })
    }

    fn recall(
        &self,
        _session_key: &str,
        id: &str,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<Option<SpillEntry>, crate::domain::error::DomainError>>
                + Send
                + '_,
        >,
    > {
        let found = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|e| e.id == id)
            .cloned();
        Box::pin(async move { Ok(found) })
    }

    fn list_entries(&self, _session_key: &str) -> crate::domain::session::SpillIndexList<'_> {
        let index: Vec<SpillIndex> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .map(|e| SpillIndex {
                id: e.id.clone(),
                tool: e.tool.clone(),
                input_preview: e.input_preview.clone(),
                tokens: e.tokens,
            })
            .collect();
        Box::pin(async move { Ok(Arc::new(index)) })
    }

    fn clear(
        &self,
        _session_key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::domain::error::DomainError>> + Send + '_>>
    {
        self.entries.lock().unwrap().clear();
        Box::pin(async { Ok(()) })
    }
}

/// An old (previous-prompt) conversation message on `turn`, already spilled at
/// creation (spill_id stamped), as production guarantees after #1046 AC1.
fn conv_msg(role: Role, turn: u32, i: u32) -> Message {
    let content = format!("conversation message {i} {}", "padding ".repeat(20));
    let mut m = match role {
        Role::Assistant => Message::assistant(&content, vec![]),
        _ => Message::user(&content),
    };
    m.turn = Some(turn);
    m.spill_id = Some(format!("turn{turn}:msg:{}", role.as_str()));
    m
}

/// A session of `n` old conversation messages (alternating user/assistant on
/// distinct old turns) followed by the in-flight (turn-less) user prompt.
fn session_with_old_conv_messages(n: u32) -> Vec<Message> {
    let mut messages: Vec<Message> = (1..=n)
        .map(|i| {
            let role = if i % 2 == 0 {
                Role::Assistant
            } else {
                Role::User
            };
            conv_msg(role, i, i)
        })
        .collect();
    messages.push(Message::user("current question"));
    messages
}

fn tool_call_msg(i: u32) -> Message {
    let mut m = Message::tool(format!("call_{i}"), format!("output {i}"));
    m.turn = Some(i);
    m.tool_name = Some("bash".to_string());
    m.spill_id = Some(format!("turn{i}:bash:0"));
    m
}

// --- stub format (AC2) ---

#[test]
fn message_stub_contains_role_preview_tokens_and_recall_id() {
    let stub = message_collapse_stub(
        "assistant",
        "I analysed the codebase and",
        840,
        "turn12:msg:assistant",
    );
    assert!(
        stub.contains("[assistant:"),
        "stub must name the role: {stub}"
    );
    assert!(
        stub.contains("I analysed the codebase"),
        "stub must carry a preview: {stub}"
    );
    assert!(
        stub.contains("840 tokens"),
        "stub must carry tokens: {stub}"
    );
    assert!(
        stub.contains("recall(\"turn12:msg:assistant\")"),
        "stub must carry the recall id: {stub}"
    );
    assert!(!stub.contains('\n'), "stub must be a one-liner: {stub:?}");
}

// --- collapse trigger semantics (AC2, AC7: N+1 not N) ---

#[test]
fn message_collapse_triggers_at_one_past_threshold_not_at_it() {
    // Exactly N live conversation messages → nothing collapses.
    let mut messages = session_with_old_conv_messages(3);
    let collapsed = collapse_conversation_messages_over_limit(&mut messages, 3, 0);
    assert_eq!(collapsed, 0, "at exactly N no message may collapse");
    assert!(messages.iter().all(|m| !m.is_collapsed));

    // Positive control: the SAME session with one more conversation message
    // must collapse exactly one — distinguishing the real trigger from a no-op.
    let mut messages = session_with_old_conv_messages(4);
    let collapsed = collapse_conversation_messages_over_limit(&mut messages, 3, 0);
    assert_eq!(collapsed, 1, "at N+1 exactly the oldest must collapse");
    assert!(messages[0].is_collapsed, "the oldest must be the stub");
    assert!(
        messages[0].content.contains("recall(\"turn1:msg:user\")"),
        "collapsed content must be a recall() stub, got: {}",
        messages[0].content
    );
    assert!(
        messages[1..].iter().all(|m| !m.is_collapsed),
        "only the oldest may collapse at N+1"
    );
}

#[test]
fn assistant_and_user_messages_share_one_combined_count() {
    // 2 user + 2 assistant old messages count as 4 combined; N=3 → 1 collapse.
    let mut messages = vec![
        conv_msg(Role::User, 1, 1),
        conv_msg(Role::Assistant, 1, 2),
        conv_msg(Role::User, 2, 3),
        conv_msg(Role::Assistant, 2, 4),
        Message::user("current question"),
    ];
    let collapsed = collapse_conversation_messages_over_limit(&mut messages, 3, 0);
    assert_eq!(
        collapsed, 1,
        "assistant + user must count as ONE combined message count"
    );
    assert!(messages[0].is_collapsed, "oldest-first ordering");
}

#[test]
fn tool_results_are_excluded_from_the_message_count() {
    // 3 conversation messages at N=3 must not collapse, no matter how many
    // tool results share the session — the dials are independent.
    let mut messages = session_with_old_conv_messages(3);
    for i in 1..=10 {
        messages.insert(0, tool_call_msg(i));
    }
    let collapsed = collapse_conversation_messages_over_limit(&mut messages, 3, 0);
    assert_eq!(collapsed, 0, "tool results must not count toward N");
    assert!(
        messages.iter().all(|m| !m.is_collapsed),
        "no message (tool or conversation) may collapse"
    );

    // Positive control: one more conversation message trips the trigger, and
    // it must collapse a conversation message — never a tool result.
    let mut messages = session_with_old_conv_messages(4);
    for i in 1..=10 {
        messages.insert(0, tool_call_msg(i));
    }
    let collapsed = collapse_conversation_messages_over_limit(&mut messages, 3, 0);
    assert_eq!(collapsed, 1);
    assert!(
        messages
            .iter()
            .filter(|m| m.role == Role::Tool)
            .all(|m| !m.is_collapsed),
        "the message trigger must never collapse tool results"
    );
}

#[test]
fn message_collapse_is_cumulative_across_prompts() {
    // Two prompts' worth of history: 4 old conversation messages from an
    // earlier prompt (turns restart each prompt) plus 4 more and the in-flight
    // prompt. 8 live conversation messages, N=3 → oldest 5 collapse.
    let mut messages: Vec<Message> = (1..=4).map(|i| conv_msg(Role::Assistant, i, i)).collect();
    messages.push(conv_msg(Role::User, 1, 5)); // later prompt, turns reset
    messages.extend((1..=3).map(|i| conv_msg(Role::Assistant, i, i + 5)));
    messages.push(Message::user("current question"));
    let collapsed = collapse_conversation_messages_over_limit(&mut messages, 3, 0);
    assert_eq!(
        collapsed, 5,
        "the trigger must count live conversation messages across prompts"
    );
    for m in &messages[..5] {
        assert!(m.is_collapsed, "the oldest 5 must be stubs");
    }
}

#[test]
fn message_collapse_disabled_by_sentinel() {
    let mut messages = session_with_old_conv_messages(100);
    let collapsed = collapse_conversation_messages_over_limit(&mut messages, COLLAPSE_DISABLED, 0);
    assert_eq!(collapsed, 0, "u32::MAX must disable message collapse");
    assert!(messages.iter().all(|m| !m.is_collapsed));

    // Control: the identical session DOES collapse at a finite limit.
    let collapsed = collapse_conversation_messages_over_limit(&mut messages, 50, 0);
    assert_eq!(collapsed, 50);
}

// --- exemptions (AC3) ---

#[test]
fn exempt_messages_never_collapse() {
    let mut manifest = Message::system("[Session memory: 1 spilled entries via recall()]");
    manifest.is_pinned = true;
    manifest.is_manifest = true;
    let mut tail_pinned = conv_msg(Role::Assistant, 9, 9);
    // Belongs to the current prompt's most recent turn.
    tail_pinned.turn = Some(9);
    let mut messages = vec![
        Message::system("system prompt"),
        manifest,
        conv_msg(Role::Assistant, 1, 1),   // old prompt — collapsible
        conv_msg(Role::User, 2, 2),        // old prompt — collapsible
        Message::user("current question"), // in-flight prompt — exempt
    ];
    // The tail-pinned message sits after the in-flight prompt (current prompt
    // region) within the pin_recent_turns tail.
    messages.push(tail_pinned);

    // N=0: everything countable must collapse — only exemptions survive.
    let collapsed = collapse_conversation_messages_over_limit(&mut messages, 0, 1);
    assert!(
        collapsed >= 1,
        "positive control: old conversation messages must collapse at N=0"
    );
    let by_content = |needle: &str| {
        messages
            .iter()
            .find(|m| m.content.contains(needle))
            .unwrap_or_else(|| panic!("message containing {needle:?} must not be dropped"))
    };
    assert!(
        !by_content("system prompt").is_collapsed,
        "the system prompt is exempt"
    );
    assert!(
        !by_content("Session memory").is_collapsed,
        "the manifest is exempt"
    );
    assert!(
        !by_content("current question").is_collapsed,
        "the in-flight user prompt is exempt"
    );
    assert!(
        !by_content("conversation message 9").is_collapsed,
        "messages within the pin_recent_turns tail are exempt"
    );
    assert!(
        by_content("conversation message 1").is_collapsed,
        "old-prompt messages must collapse at N=0"
    );
}

// --- stubs count toward the budget (AC4) ---

#[test]
fn collapsed_message_stubs_count_toward_the_token_budget() {
    let mut messages = session_with_old_conv_messages(2);
    let original_tokens = estimate_message_tokens(&messages[0]);
    let collapsed = collapse_conversation_messages_over_limit(&mut messages, 0, 0);
    assert!(collapsed >= 1, "positive control: something must collapse");
    let stub = &messages[0];
    assert!(stub.is_collapsed);
    let stub_tokens = estimate_message_tokens(stub);
    assert!(
        stub_tokens > 0,
        "a stub must contribute a nonzero token estimate to the budget"
    );
    assert!(
        stub_tokens < original_tokens,
        "the stub must be cheaper than the original ({stub_tokens} vs {original_tokens})"
    );
}

// --- independence of the two count triggers (AC7) ---

#[test]
fn message_stubs_do_not_disturb_the_tool_collapse_trigger() {
    // Collapse conversation messages first, marking them is_collapsed. The
    // tool trigger must still see 51 live tool results and collapse exactly 1.
    let mut messages = session_with_old_conv_messages(4);
    let collapsed = collapse_conversation_messages_over_limit(&mut messages, 3, 0);
    assert_eq!(collapsed, 1, "positive control: one message stub exists");
    for i in 1..=51 {
        messages.push(tool_call_msg(i));
    }
    let tool_collapsed = collapse_tool_results_over_limit(&mut messages, 50);
    assert_eq!(
        tool_collapsed, 1,
        "message stubs must not count toward the tool-call trigger"
    );
}

// --- demotion ladder (AC6) ---

/// Old conversation messages with spill ids (already on disk) + prompt.
fn spillable_session(n: u32) -> Vec<Message> {
    session_with_old_conv_messages(n)
}

#[test]
fn ladder_collapses_to_stubs_before_dropping_anything() {
    // ~40 tokens/message stubbed vs ~45 full; budget met by stubbing alone.
    let mut messages = spillable_session(4);
    let full_total = estimate_total_tokens(&messages);
    let budget = full_total - 20; // slightly over budget: stubs suffice
    let outcome = enforce_context_ceiling_ladder(&mut messages, budget, 0);
    assert!(
        outcome.collapsed_to_stubs >= 1,
        "the first rung must demote full messages to stubs"
    );
    assert_eq!(
        outcome.dropped, 0,
        "nothing may be hard-dropped while stubbing suffices"
    );
    assert!(
        estimate_total_tokens(&messages) <= budget,
        "the budget must be met"
    );
    assert!(
        messages[0].is_collapsed && messages[0].content.contains("recall("),
        "oldest-first: the oldest message becomes a recall stub"
    );
    assert!(
        !outcome.over_budget,
        "control: a met budget must not be reported as unmet"
    );
}

#[test]
fn ladder_drops_stubs_only_when_stubbing_is_insufficient() {
    let mut messages = spillable_session(4);
    // Budget so tight that even all-stubs exceeds it: the second rung must
    // remove stubs entirely (content already on disk — manifest-only).
    let before = messages.len();
    let outcome = enforce_context_ceiling_ladder(&mut messages, 5, 0);
    assert!(
        outcome.dropped >= 1,
        "still-over-budget after stubbing must drop stubs entirely"
    );
    assert!(messages.len() < before, "dropped stubs leave the context");
    assert!(
        messages
            .iter()
            .filter(|m| m.role != Role::System && m.turn.is_some())
            .all(|m| m.is_collapsed),
        "no full un-collapsed old message may survive while stubs were dropped"
    );
}

#[test]
fn ladder_never_demotes_pinned_or_exempt_and_reports_unmet_budget() {
    let mut manifest = Message::system("[Session memory: 1 spilled entries via recall()]");
    manifest.is_pinned = true;
    manifest.is_manifest = true;
    let mut messages = vec![
        Message::system("system prompt"),
        manifest,
        Message::user("current question that is quite long indeed"),
    ];
    // Budget of 1 token: unmeetable — the pinned set alone exceeds it.
    let outcome = enforce_context_ceiling_ladder(&mut messages, 1, 2);
    assert_eq!(messages.len(), 3, "pinned/exempt messages must all survive");
    assert!(
        messages.iter().all(|m| !m.is_collapsed),
        "pinned/exempt content is never demoted at any rung"
    );
    assert!(
        outcome.over_budget,
        "an unmeetable ceiling must be reported so callers can warn/audit (#1044)"
    );
}

// --- creation-time spilling (AC1) ---

#[tokio::test]
async fn spill_conversation_message_appends_full_content_and_stamps_id() {
    let store = MemStore::default();
    let mut msg = Message::assistant("the full assistant reply text", vec![]);
    msg.turn = Some(3);
    spill_conversation_message(&mut msg, &store, "s").await;
    assert_eq!(
        msg.spill_id.as_deref(),
        Some("turn3:msg:assistant"),
        "the message must be stamped with its spill id at creation"
    );
    let entry = store
        .recall("s", "turn3:msg:assistant")
        .await
        .unwrap()
        .expect("the message must be recallable immediately after creation");
    assert_eq!(entry.content, "the full assistant reply text");
    assert_eq!(entry.tool, "assistant", "spills carry the role");
    assert_eq!(
        msg.content, "the full assistant reply text",
        "spilling must not disturb the in-context content"
    );
}

#[tokio::test]
async fn spill_conversation_message_dedups_ids_across_prompts() {
    // Turn numbering restarts each prompt: two turn-1 assistant replies in one
    // session must get distinct, individually recallable ids.
    let store = MemStore::default();
    let mut first = Message::assistant("prompt A reply", vec![]);
    first.turn = Some(1);
    spill_conversation_message(&mut first, &store, "s").await;
    let mut second = Message::assistant("prompt B reply", vec![]);
    second.turn = Some(1);
    spill_conversation_message(&mut second, &store, "s").await;
    assert_eq!(second.spill_id.as_deref(), Some("turn1:msg:assistant:2"));
    let entry = store
        .recall("s", "turn1:msg:assistant:2")
        .await
        .unwrap()
        .expect("the deduplicated id must be recallable");
    assert_eq!(entry.content, "prompt B reply");
}

// --- live counting on re-entry (AC2: stubs are not "live") ---

#[test]
fn already_collapsed_stubs_are_excluded_from_the_live_count() {
    // 3 stubs + 3 full messages at N=3: the stubs must not count as live,
    // so nothing further collapses on re-entry.
    let mut messages = session_with_old_conv_messages(6);
    let collapsed = collapse_conversation_messages_over_limit(&mut messages, 3, 0);
    assert_eq!(collapsed, 3, "positive control: first pass stubs 3");
    let collapsed = collapse_conversation_messages_over_limit(&mut messages, 3, 0);
    assert_eq!(
        collapsed, 0,
        "stubs must be excluded from the live count on re-entry"
    );

    // One more full message arrives: exactly one more (the oldest FULL
    // message, never a stub) collapses.
    let idx = messages.len() - 1; // insert before the in-flight prompt
    messages.insert(idx, conv_msg(Role::Assistant, 7, 7));
    let collapsed = collapse_conversation_messages_over_limit(&mut messages, 3, 0);
    assert_eq!(collapsed, 1);
    assert!(
        messages[3].is_collapsed && messages[3].content.contains("recall("),
        "the oldest full message is the one that collapses"
    );
}

// --- ladder boundary: at/under budget is a no-op (AC6) ---

#[test]
fn ladder_is_a_no_op_at_or_under_budget() {
    let mut messages = spillable_session(4);
    let full_total = estimate_total_tokens(&messages);
    let before: Vec<String> = messages.iter().map(|m| m.content.clone()).collect();
    let outcome = enforce_context_ceiling_ladder(&mut messages, full_total, 0);
    assert_eq!(outcome.collapsed_to_stubs, 0, "at budget nothing may stub");
    assert_eq!(outcome.dropped, 0, "at budget nothing may drop");
    assert!(!outcome.over_budget);
    let after: Vec<String> = messages.iter().map(|m| m.content.clone()).collect();
    assert_eq!(before, after, "messages must be untouched at budget");
}

// --- pin_recent_turns tail on the current run's turn-stamped messages (AC3) ---

#[test]
fn current_run_recent_turns_are_exempt_from_message_collapse() {
    // Old-prompt messages (turn-stamped, before the in-flight prompt), the
    // prompt, then the current run's turns 1..=3. With pin_recent_turns=2 and
    // N=0, turns 2 and 3 are tail-pinned; turn 1 of the current run and every
    // old-prompt message collapse.
    let mut messages = vec![
        conv_msg(Role::Assistant, 1, 1), // earlier prompt — collapsible
        conv_msg(Role::Assistant, 2, 2), // earlier prompt — collapsible
        Message::user("current question"),
    ];
    for turn in 1..=3u32 {
        let mut m = Message::assistant(format!("current run answer {turn}"), vec![]);
        m.turn = Some(turn);
        // Spilled at creation, as production guarantees (#1046 AC1).
        m.spill_id = Some(format!("turn{turn}:msg:assistant"));
        messages.push(m);
    }
    let collapsed = collapse_conversation_messages_over_limit(&mut messages, 0, 2);
    assert_eq!(
        collapsed, 3,
        "both earlier-prompt messages and the current run's turn 1 collapse"
    );
    let by_content = |needle: &str| {
        messages
            .iter()
            .find(|m| m.content.contains(needle))
            .unwrap_or_else(|| panic!("message containing {needle:?} must exist"))
    };
    assert!(by_content("conversation message 1").is_collapsed);
    assert!(by_content("conversation message 2").is_collapsed);
    assert!(
        by_content("current run answer 1").is_collapsed,
        "the current run's turn 1 is outside the pin tail of 2"
    );
    assert!(
        !by_content("current run answer 2").is_collapsed,
        "turn 2 is within the pin_recent_turns tail"
    );
    assert!(
        !by_content("current run answer 3").is_collapsed,
        "turn 3 is within the pin_recent_turns tail"
    );
}

// --- ephemeral sessions (empty key): creation spilling must still persist ---
// PR #1048 follow-up: the empty-key guard in the conversation spill writer is
// removed so ephemeral runs match tool spilling (deliberately unguarded, see
// the NOTE in agent_loop_spill.rs) — collapse/ladder recall() stubs must stay
// resolvable in `--no-session` runs instead of pointing at nothing.

#[tokio::test]
async fn spill_conversation_message_persists_for_ephemeral_sessions() {
    let store = MemStore::default();
    let mut msg = Message::assistant("ephemeral reply text", vec![]);
    msg.turn = Some(1);
    let written = spill_conversation_message(&mut msg, &store, "").await;
    assert!(
        written,
        "an ephemeral (empty-key) session must still spill conversation \
         messages so later collapse stubs are recallable"
    );
    assert_eq!(
        msg.spill_id.as_deref(),
        Some("turn1:msg:assistant"),
        "the spill id must be stamped for ephemeral sessions too"
    );
    let entry = store
        .recall("", "turn1:msg:assistant")
        .await
        .unwrap()
        .expect("the ephemeral spill entry must be recallable");
    assert_eq!(entry.content, "ephemeral reply text");
}

// --- ladder rung 1: tiny messages whose stub is not cheaper are skipped ---

#[test]
fn ladder_skips_tiny_messages_whose_stub_would_not_be_cheaper() {
    // A tiny old message (stub estimate >= content estimate) among large ones.
    let mut tiny = Message::user("ok");
    tiny.turn = Some(1);
    tiny.spill_id = Some("turn1:msg:user".into());
    let mut messages = vec![tiny];
    for i in 2..=4u32 {
        messages.push(conv_msg(Role::Assistant, i, i));
    }
    messages.push(Message::user("current question"));
    let total = estimate_total_tokens(&messages);
    // Slightly over budget: stubbing the large messages suffices, so the
    // second rung never runs and the tiny message's fate is rung 1's alone.
    let budget = total - 20;
    let outcome = enforce_context_ceiling_ladder(&mut messages, budget, 0);
    assert!(
        outcome.collapsed_to_stubs >= 1,
        "positive control: large messages must be stubbed"
    );
    assert_eq!(outcome.dropped, 0, "budget must be met by stubbing alone");
    assert!(
        !messages[0].is_collapsed && messages[0].content == "ok",
        "a message whose stub would be no cheaper than its content must be \
         skipped at rung 1, not inflated into a larger stub; got: {}",
        messages[0].content
    );
    assert!(
        estimate_total_tokens(&messages) <= budget,
        "the budget must still be met"
    );
}

// --- ladder rung 2: drop order is oldest-first ---

#[test]
fn ladder_second_rung_drops_the_oldest_stub_first() {
    // All old messages are already stubs of equal size; the budget is met
    // after removing exactly ONE of them — the oldest must be the one gone.
    let mut messages = session_with_old_conv_messages(3);
    let n_old = 3;
    for m in messages[..n_old].iter_mut() {
        let tokens = estimate_tokens(&m.content);
        let id = m.spill_id.clone().unwrap();
        m.content = message_collapse_stub(m.role.as_str(), &m.content, tokens, &id);
        m.is_collapsed = true;
        m.invalidate_token_cache();
    }
    let total = estimate_total_tokens(&messages);
    let one_stub = estimate_message_tokens(&messages[0]);
    // Removing one stub meets the budget; removing two would be over-pruning.
    let budget = total - 1;
    assert!(
        one_stub > 1,
        "sanity: a stub removal must free enough tokens"
    );
    let outcome = enforce_context_ceiling_ladder(&mut messages, budget, 0);
    assert_eq!(
        outcome.dropped, 1,
        "exactly one stub removal suffices for this budget"
    );
    let contents: Vec<&str> = messages.iter().map(|m| m.content.as_str()).collect();
    assert!(
        !contents.iter().any(|c| c.contains("turn1:msg:user")),
        "the OLDEST stub (turn1) must be the one removed, got: {contents:?}"
    );
    assert!(
        contents.iter().any(|c| c.contains("turn2:msg:assistant"))
            && contents.iter().any(|c| c.contains("turn3:msg:user")),
        "newer stubs must survive a partial drop, got: {contents:?}"
    );
}

// --- creation-spill id dedup: third collision mints :3, not a re-used :2 ---

#[tokio::test]
async fn creation_spill_third_collision_mints_suffix_3() {
    let store = MemStore::default();
    for (i, text) in ["prompt A reply", "prompt B reply", "prompt C reply"]
        .iter()
        .enumerate()
    {
        let mut msg = Message::assistant(*text, vec![]);
        msg.turn = Some(1);
        spill_conversation_message(&mut msg, &store, "s").await;
        let expected = match i {
            0 => "turn1:msg:assistant".to_string(),
            n => format!("turn1:msg:assistant:{}", n + 1),
        };
        assert_eq!(
            msg.spill_id.as_deref(),
            Some(expected.as_str()),
            "collision {i} must mint the next free suffix"
        );
    }
    let entry = store
        .recall("s", "turn1:msg:assistant:3")
        .await
        .unwrap()
        .expect("the third-collision id must be recallable");
    assert_eq!(
        entry.content, "prompt C reply",
        "turn1:msg:assistant:3 must recall the THIRD colliding message"
    );
}
