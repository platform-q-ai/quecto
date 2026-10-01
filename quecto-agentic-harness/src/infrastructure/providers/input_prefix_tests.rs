//! #2398: where each request's input first differs from its session's
//! previous request.
use super::*;
use crate::domain::token_estimate::estimate_tokens;
use serde_json::{Value, json};

fn user(text: &str) -> Value {
    json!({"role": "user", "content": text})
}
fn assistant(text: &str) -> Value {
    json!({"role": "assistant", "phase": "commentary", "content": text})
}
fn call(id: &str) -> Value {
    json!({"type": "function_call", "call_id": id, "name": "bash", "arguments": "{}"})
}
fn output(id: &str, text: &str) -> Value {
    json!({"type": "function_call_output", "call_id": id, "output": text})
}
fn reasoning(content: &str) -> Value {
    json!({"type": "reasoning", "summary": [], "encrypted_content": content})
}

/// The estimated tokens of `items`, each as it is serialized.
fn tokens(items: &[Value]) -> usize {
    items
        .iter()
        .map(|item| estimate_tokens(&serde_json::to_string(item).unwrap()))
        .sum()
}

#[test]
fn a_sessions_first_request_has_an_empty_unchanged_prefix() {
    let digests = InputDigests::default();
    let input = [user("hi"), assistant("hello")];
    assert_eq!(
        digests.observe(Some("s"), &input),
        InputPrefix {
            input_items: 2,
            first_changed_item: None,
            first_changed_kind: None,
            prefix_tokens_estimate: 0,
        }
    );
}

#[test]
fn an_append_only_sequence_records_no_changed_item() {
    let digests = InputDigests::default();
    let mut input = vec![user("list the files")];
    digests.observe(Some("s"), &input);
    for next in [
        call("c1"),
        output("c1", "a.rs b.rs"),
        assistant("two files"),
    ] {
        let previous = input.clone();
        input.push(next);
        let observed = digests.observe(Some("s"), &input);
        assert_eq!(observed.input_items, input.len());
        assert_eq!(observed.first_changed_item, None, "{input:?}");
        assert_eq!(observed.first_changed_kind, None);
        assert_eq!(observed.prefix_tokens_estimate, tokens(&previous));
    }
}

#[test]
fn an_identical_resend_is_append_only() {
    let digests = InputDigests::default();
    let input = [user("hi"), call("c1"), output("c1", "ok")];
    digests.observe(None, &input);
    let observed = digests.observe(None, &input);
    assert_eq!(observed.first_changed_item, None);
    assert_eq!(observed.prefix_tokens_estimate, tokens(&input));
}

#[test]
fn an_in_place_edit_of_item_n_records_n_and_its_kind() {
    let digests = InputDigests::default();
    let before = [
        user("go"),
        call("c1"),
        output("c1", "long output"),
        assistant("done"),
    ];
    digests.observe(Some("s"), &before);
    let after = [
        user("go"),
        call("c1"),
        output("c1", "[pruned]"),
        assistant("done"),
        user("again"),
    ];
    assert_eq!(
        digests.observe(Some("s"), &after),
        InputPrefix {
            input_items: 5,
            first_changed_item: Some(2),
            first_changed_kind: Some(InputItemKind::FunctionCallOutput),
            prefix_tokens_estimate: tokens(&after[..2]),
        }
    );
}

#[test]
fn an_edited_item_of_each_kind_records_that_kind() {
    let cases = [
        (user("b"), InputItemKind::User),
        (assistant("b"), InputItemKind::Assistant),
        (call("b"), InputItemKind::FunctionCall),
        (output("c", "b"), InputItemKind::FunctionCallOutput),
        (reasoning("b"), InputItemKind::Reasoning),
    ];
    let originals = [
        user("a"),
        assistant("a"),
        call("a"),
        output("c", "a"),
        reasoning("a"),
    ];
    for ((edited, kind), original) in cases.into_iter().zip(originals) {
        let digests = InputDigests::default();
        digests.observe(Some("s"), &[user("first"), original]);
        let observed = digests.observe(Some("s"), &[user("first"), edited]);
        assert_eq!(observed.first_changed_item, Some(1), "{kind:?}");
        assert_eq!(observed.first_changed_kind, Some(kind));
    }
}

#[test]
fn a_removed_item_records_the_index_where_the_lists_diverge() {
    let digests = InputDigests::default();
    digests.observe(Some("s"), &[user("1"), user("2"), user("3"), user("4")]);
    let after = [user("1"), user("3"), user("4")];
    assert_eq!(
        digests.observe(Some("s"), &after),
        InputPrefix {
            input_items: 3,
            first_changed_item: Some(1),
            first_changed_kind: Some(InputItemKind::User),
            prefix_tokens_estimate: tokens(&after[..1]),
        }
    );
}

/// The previous request's input is longer than this one, which it starts
/// with: the first item this request lacks is where they diverge, and it
/// has no kind here.
#[test]
fn a_truncated_input_records_its_first_missing_index_without_a_kind() {
    let digests = InputDigests::default();
    digests.observe(Some("s"), &[user("1"), call("c"), output("c", "x")]);
    let after = [user("1"), call("c")];
    assert_eq!(
        digests.observe(Some("s"), &after),
        InputPrefix {
            input_items: 2,
            first_changed_item: Some(2),
            first_changed_kind: None,
            prefix_tokens_estimate: tokens(&after),
        }
    );
}

#[test]
fn each_session_is_compared_only_with_its_own_previous_request() {
    let digests = InputDigests::default();
    digests.observe(Some("one"), &[user("a")]);
    digests.observe(Some("two"), &[user("b")]);
    digests.observe(None, &[user("c")]);
    let observed = digests.observe(Some("one"), &[user("a"), assistant("x")]);
    assert_eq!(observed.first_changed_item, None);
    assert_eq!(observed.prefix_tokens_estimate, tokens(&[user("a")]));
    let observed = digests.observe(None, &[user("c"), user("d")]);
    assert_eq!(observed.first_changed_item, None);
    assert_eq!(observed.prefix_tokens_estimate, tokens(&[user("c")]));
}

/// Only the most recently sent sessions are kept: the least recent is
/// forgotten, and its next request reads as its first.
#[test]
fn the_least_recently_sent_session_is_forgotten_beyond_the_bound() {
    let digests = InputDigests::default();
    for session in 0..=SESSIONS_RETAINED {
        digests.observe(Some(&session.to_string()), &[user("a")]);
    }
    let input = [user("a"), user("b")];
    let forgotten = digests.observe(Some("0"), &input);
    assert_eq!(forgotten.first_changed_item, None);
    assert_eq!(forgotten.prefix_tokens_estimate, 0);
    let kept = digests.observe(Some(&SESSIONS_RETAINED.to_string()), &input);
    assert_eq!(kept.prefix_tokens_estimate, tokens(&input[..1]));
    assert!(digests.sessions.lock().unwrap().len() <= SESSIONS_RETAINED);
}

#[test]
fn a_shape_outside_the_known_kinds_has_no_kind() {
    assert_eq!(
        kind(&json!({"type": "message", "role": "user"})),
        Some(InputItemKind::User)
    );
    assert_eq!(kind(&json!({"type": "web_search_call"})), None);
    assert_eq!(kind(&json!({"role": "developer", "content": "x"})), None);
    assert_eq!(kind(&json!("text")), None);
    let digests = InputDigests::default();
    digests.observe(None, &[json!({"type": "custom", "v": 1})]);
    let observed = digests.observe(None, &[json!({"type": "custom", "v": 2})]);
    assert_eq!(observed.first_changed_item, Some(0));
    assert_eq!(observed.first_changed_kind, None);
}

/// The record and the kept digests carry no content.
#[test]
fn nothing_kept_or_recorded_carries_content() {
    const SECRET: &str = "sk-proj-QX7hunter2SECRETtoken9d1f";
    let digests = InputDigests::default();
    digests.observe(Some("s"), &[user(SECRET)]);
    let observed = digests.observe(
        Some("s"),
        &[
            user(&format!("{SECRET}!")),
            call(SECRET),
            output(SECRET, SECRET),
        ],
    );
    assert_eq!(observed.first_changed_item, Some(0));
    let kept = format!("{digests:?} {observed:?}");
    let recorded = serde_json::to_string(&observed).unwrap();
    for text in [kept, recorded] {
        for fragment in ["sk-proj", "hunter2", "SECRET", "9d1f"] {
            assert!(!text.contains(fragment), "{fragment} in {text}");
        }
    }
}
