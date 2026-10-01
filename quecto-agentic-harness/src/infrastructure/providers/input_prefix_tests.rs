//! #2398: where each request's input first differs from its session's
//! previous accepted request.
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

fn body(instructions: &str, input: &[Value]) -> Value {
    json!({"instructions": instructions, "input": input, "model": "m"})
}

/// The estimated tokens of `items`, each as it is serialized.
fn tokens(items: &[Value]) -> usize {
    items
        .iter()
        .map(|item| estimate_tokens(&serde_json::to_string(item).unwrap()))
        .sum()
}

/// The estimated tokens of a body's instructions (and absent tools).
fn head(instructions: &str) -> usize {
    estimate_tokens(&json!([instructions, null]).to_string())
}

/// Compare a request of `input` in `session` and accept it.
fn observe(digests: &InputDigests, session: &str, input: &[Value]) -> InputPrefixParts {
    let measured = MeasuredInput::of(session, &body("sys", input));
    let observed = digests.compare(&measured).parts();
    digests.commit(measured);
    observed
}

#[test]
fn a_sessions_first_request_compares_with_nothing() {
    let digests = InputDigests::default();
    let input = [user("hi"), assistant("hello")];
    assert_eq!(
        observe(&digests, "s", &input),
        InputPrefixParts {
            input_items: 2,
            previous_items: None,
            first_changed_item: None,
            first_changed_kind: None,
            prefix_tokens_estimate: 0,
            unchanged_prefix_tokens_estimate: 0,
            request_tokens_estimate: head("sys") + tokens(&input),
        }
    );
}

#[test]
fn an_append_only_sequence_records_no_changed_item() {
    let digests = InputDigests::default();
    let mut input = vec![user("list the files")];
    observe(&digests, "s", &input);
    for next in [
        call("c1"),
        output("c1", "a.rs b.rs"),
        assistant("two files"),
    ] {
        let previous = input.clone();
        input.push(next);
        assert_eq!(
            observe(&digests, "s", &input),
            InputPrefixParts {
                input_items: input.len(),
                previous_items: Some(previous.len()),
                first_changed_item: None,
                first_changed_kind: None,
                prefix_tokens_estimate: tokens(&previous),
                unchanged_prefix_tokens_estimate: head("sys") + tokens(&previous),
                request_tokens_estimate: head("sys") + tokens(&input),
            }
        );
    }
}

#[test]
fn an_identical_resend_is_append_only() {
    let digests = InputDigests::default();
    let input = [user("hi"), call("c1"), output("c1", "ok")];
    observe(&digests, "s", &input);
    let observed = observe(&digests, "s", &input);
    assert_eq!(observed.first_changed_item, None);
    assert_eq!(observed.previous_items, Some(3));
    assert_eq!(observed.prefix_tokens_estimate, tokens(&input));
    assert_eq!(
        observed.unchanged_prefix_tokens_estimate,
        observed.request_tokens_estimate
    );
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
    observe(&digests, "s", &before);
    let after = [
        user("go"),
        call("c1"),
        output("c1", "[pruned]"),
        assistant("done"),
        user("again"),
    ];
    assert_eq!(
        observe(&digests, "s", &after),
        InputPrefixParts {
            input_items: 5,
            previous_items: Some(4),
            first_changed_item: Some(2),
            first_changed_kind: Some(InputItemKind::FunctionCallOutput),
            prefix_tokens_estimate: tokens(&after[..2]),
            unchanged_prefix_tokens_estimate: head("sys") + tokens(&after[..2]),
            request_tokens_estimate: head("sys") + tokens(&after),
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
        observe(&digests, "s", &[user("first"), original]);
        let observed = observe(&digests, "s", &[user("first"), edited]);
        assert_eq!(observed.first_changed_item, Some(1), "{kind:?}");
        assert_eq!(observed.first_changed_kind, Some(kind));
    }
}

#[test]
fn a_removed_item_records_the_index_where_the_lists_diverge() {
    let digests = InputDigests::default();
    observe(&digests, "s", &[user("1"), user("2"), user("3"), user("4")]);
    let after = [user("1"), user("3"), user("4")];
    let observed = observe(&digests, "s", &after);
    assert_eq!(observed.previous_items, Some(4));
    assert_eq!(observed.first_changed_item, Some(1));
    assert_eq!(observed.first_changed_kind, Some(InputItemKind::User));
    assert_eq!(observed.prefix_tokens_estimate, tokens(&after[..1]));
}

/// The previous request's input is longer than this one, which it starts
/// with: the first item this request lacks is where they diverge, and it
/// has no kind here.
#[test]
fn a_truncated_input_records_its_first_missing_index_without_a_kind() {
    let digests = InputDigests::default();
    observe(&digests, "s", &[user("1"), call("c"), output("c", "x")]);
    let after = [user("1"), call("c")];
    let observed = observe(&digests, "s", &after);
    assert_eq!(observed.first_changed_item, Some(2));
    assert_eq!(observed.first_changed_kind, None);
    assert_eq!(observed.prefix_tokens_estimate, tokens(&after));
}

/// Changed instructions leave no unchanged prefix of the whole request,
/// whatever the items kept.
#[test]
fn changed_instructions_leave_no_unchanged_whole_prefix() {
    let digests = InputDigests::default();
    let input = [user("a")];
    let first = MeasuredInput::of("s", &body("one", &input));
    digests.compare(&first);
    digests.commit(first);
    let appended = [user("a"), user("b")];
    let observed = digests
        .compare(&MeasuredInput::of("s", &body("two", &appended)))
        .parts();
    assert_eq!(observed.first_changed_item, None);
    assert_eq!(observed.prefix_tokens_estimate, tokens(&input));
    assert_eq!(observed.unchanged_prefix_tokens_estimate, 0);
    assert_eq!(
        observed.request_tokens_estimate,
        head("two") + tokens(&appended)
    );
}

/// A request compared but never accepted (a failed or cancelled send) does
/// not become the baseline.
#[test]
fn only_an_accepted_request_becomes_the_baseline() {
    let digests = InputDigests::default();
    observe(&digests, "s", &[user("a")]);
    let edited = MeasuredInput::of("s", &body("sys", &[user("EDITED")]));
    assert_eq!(digests.compare(&edited).parts().first_changed_item, Some(0));
    let observed = observe(&digests, "s", &[user("a"), user("b")]);
    assert_eq!(observed.previous_items, Some(1));
    assert_eq!(observed.first_changed_item, None);
}

/// Another body of the same request (resent without replayed reasoning)
/// is measured for the same session.
#[test]
fn another_body_of_a_request_keeps_its_session() {
    let digests = InputDigests::default();
    let replaying = MeasuredInput::of("s", &body("sys", &[user("a"), reasoning("r")]));
    digests.commit(replaying.for_body(&body("sys", &[user("a")])));
    let observed = observe(&digests, "s", &[user("a"), user("b")]);
    assert_eq!(observed.previous_items, Some(1));
    assert_eq!(observed.first_changed_item, None);
}

#[test]
fn each_session_is_compared_only_with_its_own_previous_request() {
    let digests = InputDigests::default();
    observe(&digests, "one", &[user("a")]);
    observe(&digests, "two", &[user("b"), user("c")]);
    let observed = observe(&digests, "one", &[user("a"), assistant("x")]);
    assert_eq!(observed.previous_items, Some(1));
    assert_eq!(observed.first_changed_item, None);
    assert_eq!(observed.prefix_tokens_estimate, tokens(&[user("a")]));
}

/// Only the most recently accepted sessions are kept: the least recent is
/// forgotten, and its next request compares with nothing. Accepting a
/// request again makes its session the most recent.
#[test]
fn the_least_recently_accepted_session_is_forgotten_beyond_the_bound() {
    let digests = InputDigests::default();
    for session in 0..SESSIONS_RETAINED {
        observe(&digests, &session.to_string(), &[user("a")]);
    }
    observe(&digests, "0", &[user("a")]);
    observe(&digests, "new", &[user("a")]);
    assert_eq!(digests.sessions.lock().unwrap().len(), SESSIONS_RETAINED);
    let compared = |session: &str| {
        digests
            .compare(&MeasuredInput::of(session, &body("sys", &[user("a")])))
            .parts()
            .previous_items
    };
    assert_eq!(compared("1"), None, "the least recent is forgotten");
    assert_eq!(compared("0"), Some(1), "re-accepted, so kept");
    assert_eq!(compared("2"), Some(1));
    assert_eq!(compared("new"), Some(1));
}

/// Every provider of the process compares with the same baselines.
#[test]
fn the_shared_baselines_are_one_store() {
    assert!(Arc::ptr_eq(
        &InputDigests::shared(),
        &InputDigests::shared()
    ));
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
    observe(&digests, "s", &[json!({"type": "custom", "v": 1})]);
    let observed = observe(&digests, "s", &[json!({"type": "custom", "v": 2})]);
    assert_eq!(observed.first_changed_item, Some(0));
    assert_eq!(observed.first_changed_kind, None);
}

/// Nothing kept, measured or recorded carries content or the session key.
#[test]
fn nothing_kept_or_recorded_carries_content() {
    const SECRET: &str = "sk-proj-QX7hunter2SECRETtoken9d1f";
    let session = format!("cli:{SECRET}");
    let digests = InputDigests::default();
    observe(&digests, &session, &[user(SECRET)]);
    let input = [
        user(&format!("{SECRET}!")),
        call(SECRET),
        output(SECRET, SECRET),
    ];
    let measured = MeasuredInput::of(&session, &body(SECRET, &input));
    let observed = digests.compare(&measured);
    assert_eq!(observed.parts().first_changed_item, Some(0));
    let kept = format!("{digests:?} {measured:?} {observed:?}");
    digests.commit(measured);
    let after = format!("{digests:?}");
    let recorded = serde_json::to_string(&observed).unwrap();
    for text in [kept, after, recorded] {
        for fragment in ["sk-proj", "hunter2", "SECRET", "9d1f", "cli:"] {
            assert!(!text.contains(fragment), "{fragment} in {text}");
        }
    }
}
