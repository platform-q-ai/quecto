//! #2398, #2410: how any wire's measured requests are compared with, and
//! kept as, their session's baseline. The items here are the test's own;
//! each wire's measurement of its bodies is tested beside it.
use super::*;
use crate::domain::inference::events::request_observation::{InputBaseline, InputPrefixParts};
use serde_json::Value;

/// An item whose digest is `name`'s, of `tokens` tokens.
fn item(name: &str, tokens: usize, kind: Option<InputItemKind>) -> InputItem {
    InputItem {
        digest: digest(name.as_bytes()),
        tokens,
        kind,
    }
}

fn user(name: &str) -> InputItem {
    item(name, 10, Some(InputItemKind::User))
}

/// A request of `session` whose head is `head` and whose items are `items`.
fn request(session: &str, head: &Value, items: Vec<InputItem>) -> MeasuredInput {
    MeasuredInput::of_items(session, "endpoint", "model", &[head], items)
}

fn observe(baseline: &InputBaseline, measured: MeasuredInput) -> InputPrefixParts {
    let observed = compare(baseline, &measured).expect("consistent").parts();
    commit(baseline, measured);
    observed
}

/// A head of `head_tokens` estimated tokens.
fn head_tokens(head: &Value) -> usize {
    crate::domain::inference::value_objects::token_estimate::estimate_tokens(&head.to_string())
}

#[test]
fn a_first_request_compares_with_nothing() {
    let baseline = InputBaseline::default();
    let head = serde_json::json!("instructions");
    let first = observe(&baseline, request("s", &head, vec![user("a"), user("b")]));
    assert_eq!(first.previous_items, None);
    assert_eq!(first.first_changed_item, None);
    assert_eq!(first.unchanged_prefix_tokens_estimate, 0);
    assert_eq!(first.request_tokens_estimate, head_tokens(&head) + 20);
}

#[test]
fn appending_keeps_the_whole_previous_request_unchanged() {
    let baseline = InputBaseline::default();
    let head = serde_json::json!("instructions");
    observe(&baseline, request("s", &head, vec![user("a")]));
    let next = observe(&baseline, request("s", &head, vec![user("a"), user("b")]));
    assert_eq!(next.previous_items, Some(1));
    assert_eq!(next.first_changed_item, None);
    assert_eq!(next.prefix_tokens_estimate, 10);
    assert_eq!(
        next.unchanged_prefix_tokens_estimate,
        head_tokens(&head) + 10
    );
}

#[test]
fn an_edit_records_where_and_what_changed_and_a_cut_records_where() {
    let baseline = InputBaseline::default();
    let head = serde_json::json!("instructions");
    let call = || item("call", 5, Some(InputItemKind::FunctionCall));
    observe(
        &baseline,
        request("s", &head, vec![user("a"), call(), user("b")]),
    );
    let edited = item("edited", 5, Some(InputItemKind::FunctionCall));
    let next = observe(
        &baseline,
        request("s", &head, vec![user("a"), edited, user("b")]),
    );
    assert_eq!(next.first_changed_item, Some(1));
    assert_eq!(next.first_changed_kind, Some(InputItemKind::FunctionCall));
    assert_eq!(next.prefix_tokens_estimate, 10);
    let cut = observe(&baseline, request("s", &head, vec![user("a")]));
    assert_eq!(cut.first_changed_item, Some(1));
    assert_eq!(cut.first_changed_kind, None, "nothing is there to name");
}

/// What precedes the input is cached first: changed, nothing is unchanged.
#[test]
fn a_changed_head_leaves_nothing_unchanged() {
    let baseline = InputBaseline::default();
    observe(
        &baseline,
        request("s", &serde_json::json!("one"), vec![user("a")]),
    );
    let next = observe(
        &baseline,
        request("s", &serde_json::json!("two"), vec![user("a")]),
    );
    assert_eq!(next.first_changed_item, None);
    assert_eq!(next.unchanged_prefix_tokens_estimate, 0);
    // A head part that is not a string or a list is digested, not counted.
    let measured = request("s", &serde_json::json!({"not": "sent"}), vec![]);
    assert_eq!(measured.head_tokens, 0);
}

#[test]
fn another_sessions_baseline_is_not_compared() {
    let baseline = InputBaseline::default();
    observe(
        &baseline,
        request("one", &serde_json::json!("h"), vec![user("a")]),
    );
    let other = observe(
        &baseline,
        request("two", &serde_json::json!("h"), vec![user("a")]),
    );
    assert_eq!(other.previous_items, None);
}

/// Only an accepted request becomes the baseline; one resent as another
/// body keeps its session and endpoint.
#[test]
fn a_pending_input_is_kept_once_accepted_as_it_was_last_sent() {
    let untraced = RequestTrace::default();
    let first = || request("s", &serde_json::json!("h"), vec![user("a"), user("r")]);
    assert!(PendingInput::begin(&untraced, first()).is_none());
    let baseline = InputBaseline::default();
    let trace = RequestTrace::default();
    trace.attach_input_baseline(baseline.clone());
    let refused = PendingInput::begin(&trace, first()).expect("a baseline");
    drop(refused);
    let trace = RequestTrace::default();
    trace.attach_input_baseline(baseline.clone());
    let pending = PendingInput::begin(&trace, first()).expect("a baseline");
    assert_eq!(
        trace
            .input_prefix()
            .map(|prefix| prefix.parts().previous_items),
        Some(None),
        "a refused send never became the baseline"
    );
    pending
        .resent(|sent| sent.with_items("model", &[&serde_json::json!("h")], vec![user("a")]))
        .accept();
    let next = observe(
        &baseline,
        request("s", &serde_json::json!("h"), vec![user("a"), user("b")]),
    );
    assert_eq!(next.previous_items, Some(1), "the resent body was kept");
    assert_eq!(next.first_changed_item, None);
}

/// A digest is of the bytes; a value's serialized form is its JSON.
#[test]
fn a_digest_is_stable_and_a_value_serializes_as_its_json() {
    assert_eq!(digest(b"a"), digest(b"a"));
    assert!(digest(b"a") != digest(b"b"));
    let mut bytes = Vec::new();
    let value = serde_json::json!({"b": [1, "x"]});
    assert_eq!(serialized(&value, &mut bytes), value.to_string());
}
