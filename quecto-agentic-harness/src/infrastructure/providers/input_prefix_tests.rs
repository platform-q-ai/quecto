//! #2398: where each request's input first differs from its session's
//! previous accepted request.
use super::*;
use crate::domain::conversation::image_tokens::{
    MIN_IMAGE_TOKENS, UNREADABLE_IMAGE_TOKENS, estimate_image_tokens,
};
use crate::domain::request_observation::{
    InputBaseline, InputItemKind, InputPrefixParts, RequestTrace,
};
use crate::domain::token_estimate::{estimate_opaque_tokens, estimate_tokens};
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

const ENDPOINT: &str = "https://h/codex/responses";

fn body(instructions: &str, input: &[Value]) -> Value {
    json!({"instructions": instructions, "input": input, "model": "m"})
}

/// The estimated tokens of `items`, each as it is serialized; reasoning at
/// the opaque rate.
fn tokens(items: &[Value]) -> usize {
    items
        .iter()
        .map(|item| {
            let text = serde_json::to_string(item).unwrap();
            match kind(item) {
                Some(InputItemKind::Reasoning) => estimate_opaque_tokens(&text),
                Some(_) | None => estimate_tokens(&text),
            }
        })
        .sum()
}

/// The estimated tokens of a body's instructions (it sends no tools).
fn head(instructions: &str) -> usize {
    estimate_tokens(&json!(instructions).to_string())
}

/// Compare a request of `input` in `session` with `baseline` and accept it.
fn observe(baseline: &InputBaseline, session: &str, input: &[Value]) -> InputPrefixParts {
    let measured = MeasuredInput::of(session, ENDPOINT, &body("sys", input));
    let observed = compare(baseline, &measured).expect("consistent").parts();
    commit(baseline, measured);
    observed
}

#[test]
fn a_sessions_first_request_compares_with_nothing() {
    let baseline = InputBaseline::default();
    let input = [user("hi"), assistant("hello")];
    assert_eq!(
        observe(&baseline, "s", &input),
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
    let baseline = InputBaseline::default();
    let mut input = vec![user("list the files")];
    observe(&baseline, "s", &input);
    for next in [
        call("c1"),
        output("c1", "a.rs b.rs"),
        assistant("two files"),
    ] {
        let previous = input.clone();
        input.push(next);
        assert_eq!(
            observe(&baseline, "s", &input),
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
    let baseline = InputBaseline::default();
    let input = [user("hi"), call("c1"), output("c1", "ok")];
    observe(&baseline, "s", &input);
    let observed = observe(&baseline, "s", &input);
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
    let baseline = InputBaseline::default();
    let before = [
        user("go"),
        call("c1"),
        output("c1", "long output"),
        assistant("done"),
    ];
    observe(&baseline, "s", &before);
    let after = [
        user("go"),
        call("c1"),
        output("c1", "[pruned]"),
        assistant("done"),
        user("again"),
    ];
    assert_eq!(
        observe(&baseline, "s", &after),
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
        let baseline = InputBaseline::default();
        observe(&baseline, "s", &[user("first"), original]);
        let observed = observe(&baseline, "s", &[user("first"), edited]);
        assert_eq!(observed.first_changed_item, Some(1), "{kind:?}");
        assert_eq!(observed.first_changed_kind, Some(kind));
    }
}

#[test]
fn a_removed_item_records_the_index_where_the_lists_diverge() {
    let baseline = InputBaseline::default();
    observe(
        &baseline,
        "s",
        &[user("1"), user("2"), user("3"), user("4")],
    );
    let after = [user("1"), user("3"), user("4")];
    let observed = observe(&baseline, "s", &after);
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
    let baseline = InputBaseline::default();
    observe(&baseline, "s", &[user("1"), call("c"), output("c", "x")]);
    let after = [user("1"), call("c")];
    let observed = observe(&baseline, "s", &after);
    assert_eq!(observed.first_changed_item, Some(2));
    assert_eq!(observed.first_changed_kind, None);
    assert_eq!(observed.prefix_tokens_estimate, tokens(&after));
}

/// Measure `input` of session `s` with `instructions` and `model`, sent to
/// `endpoint`.
fn measured(instructions: &str, model: &str, endpoint: &str, input: &[Value]) -> MeasuredInput {
    let mut sent = body(instructions, input);
    sent["model"] = json!(model);
    MeasuredInput::of("s", endpoint, &sent)
}

/// Changed instructions, model or endpoint leave no unchanged prefix of the
/// whole request, whatever the items kept.
#[test]
fn a_changed_instruction_model_or_endpoint_leaves_no_unchanged_whole_prefix() {
    let input = [user("a")];
    let appended = [user("a"), user("b")];
    for (instructions, model, endpoint) in [
        ("two", "m", ENDPOINT),
        ("one", "m2", ENDPOINT),
        ("one", "m", "https://other/responses"),
    ] {
        let baseline = InputBaseline::default();
        commit(&baseline, measured("one", "m", ENDPOINT, &input));
        let observed = compare(
            &baseline,
            &measured(instructions, model, endpoint, &appended),
        )
        .expect("consistent")
        .parts();
        assert_eq!(observed.first_changed_item, None, "{model} {endpoint}");
        assert_eq!(observed.prefix_tokens_estimate, tokens(&input));
        assert_eq!(
            observed.unchanged_prefix_tokens_estimate, 0,
            "{model} {endpoint}"
        );
        assert_eq!(
            observed.request_tokens_estimate,
            head(instructions) + tokens(&appended),
            "the model and endpoint are no tokens"
        );
    }
    let baseline = InputBaseline::default();
    commit(&baseline, measured("one", "m", ENDPOINT, &input));
    let same = compare(&baseline, &measured("one", "m", ENDPOINT, &appended))
        .expect("consistent")
        .parts();
    assert_eq!(
        same.unchanged_prefix_tokens_estimate,
        head("one") + tokens(&input)
    );
}

/// Encrypted reasoning is estimated at the opaque rate, not as dense text.
#[test]
fn reasoning_is_estimated_at_the_opaque_rate() {
    let blob = "gAAAAABpQx7Zk3VbW9qL2mN8rT5yH1cF6dJ0sE4uI7oP";
    let item = reasoning(blob);
    let text = serde_json::to_string(&item).unwrap();
    assert_ne!(estimate_opaque_tokens(&text), estimate_tokens(&text));
    let baseline = InputBaseline::default();
    observe(&baseline, "s", std::slice::from_ref(&item));
    let observed = observe(&baseline, "s", &[item, user("next")]);
    assert_eq!(
        observed.prefix_tokens_estimate,
        estimate_opaque_tokens(&text)
    );
}

/// #2421 review L5: an image is costed from its pixel size (#2420), not as
/// the base64 text of its data URL: the item's text without its images,
/// plus each image's estimate; an image whose URL is not inline data costs
/// the most an image can.
#[test]
fn an_items_images_are_estimated_from_their_pixel_size() {
    // A 1x1 PNG's header, then padding: the floor, far under its text.
    let data = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ".repeat(200);
    let image = json!({
        "type": "input_image",
        "image_url": format!("data:image/png;base64,{data}"),
        "detail": "high",
    });
    let remote = json!({"type": "input_image", "image_url": "https://e.example/a.png"});
    let prompt = json!({
        "role": "user",
        "content": [{"type": "input_text", "text": "look"}, image.clone(), image],
    });
    let result = json!({
        "type": "function_call_output",
        "call_id": "c1",
        "output": [remote],
    });
    let text_of = |item: Value| estimate_tokens(&serde_json::to_string(&item).unwrap());
    let prompt_text = text_of(json!({
        "role": "user",
        "content": [{"type": "input_text", "text": "look"}],
    }));
    let result_text = text_of(json!({
        "type": "function_call_output",
        "call_id": "c1",
        "output": [],
    }));
    let one_pixel = estimate_image_tokens(quecto_image::ImageMime::Png, &data);
    assert_eq!(one_pixel, MIN_IMAGE_TOKENS);
    let baseline = InputBaseline::default();
    observe(&baseline, "s", &[prompt.clone(), result.clone()]);
    let observed = observe(&baseline, "s", &[prompt, result, user("next")]);
    assert_eq!(
        observed.prefix_tokens_estimate,
        prompt_text + 2 * one_pixel + result_text + UNREADABLE_IMAGE_TOKENS
    );
}

/// #2423: the wire's type is read exactly; one off the allowlist (another
/// case, or another format) costs the most an image can, whatever its data.
#[test]
fn an_image_url_typed_off_the_allowlist_costs_the_ceiling() {
    let data = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ".repeat(200);
    for mime in ["image/png", "IMAGE/PNG", "image/bmp"] {
        let part = json!({
            "type": "input_image",
            "image_url": format!("data:{mime};base64,{data}"),
        });
        let expected = match mime {
            "image/png" => MIN_IMAGE_TOKENS,
            _ => UNREADABLE_IMAGE_TOKENS,
        };
        assert_eq!(image_part_tokens(&part), expected, "{mime}");
    }
}

/// A request compared but never accepted (a failed or cancelled send) does
/// not become the baseline.
#[test]
fn only_an_accepted_request_becomes_the_baseline() {
    let baseline = InputBaseline::default();
    observe(&baseline, "s", &[user("a")]);
    let edited = MeasuredInput::of("s", ENDPOINT, &body("sys", &[user("EDITED")]));
    let compared = compare(&baseline, &edited).expect("consistent").parts();
    assert_eq!(compared.first_changed_item, Some(0));
    let observed = observe(&baseline, "s", &[user("a"), user("b")]);
    assert_eq!(observed.previous_items, Some(1));
    assert_eq!(observed.first_changed_item, None);
}

/// Another body of the same request (resent without replayed reasoning)
/// is measured for the same session and endpoint.
#[test]
fn another_body_of_a_request_keeps_its_session_and_endpoint() {
    let baseline = InputBaseline::default();
    let replaying = MeasuredInput::of("s", ENDPOINT, &body("sys", &[user("a"), reasoning("r")]));
    commit(&baseline, replaying.for_body(&body("sys", &[user("a")])));
    let observed = observe(&baseline, "s", &[user("a"), user("b")]);
    assert_eq!(observed.previous_items, Some(1));
    assert_eq!(observed.first_changed_item, None);
    assert_eq!(
        observed.unchanged_prefix_tokens_estimate,
        head("sys") + tokens(&[user("a")])
    );
}

/// A baseline another session left is not this session's previous request.
#[test]
fn another_sessions_baseline_is_not_compared() {
    let baseline = InputBaseline::default();
    observe(&baseline, "one", &[user("a")]);
    let observed = observe(&baseline, "two", &[user("a"), user("b")]);
    assert_eq!(observed.previous_items, None);
    assert_eq!(observed.prefix_tokens_estimate, 0);
    let observed = observe(&baseline, "two", &[user("a"), user("b"), user("c")]);
    assert_eq!(observed.previous_items, Some(2));
}

/// A request is measured, compared and recorded through its trace, which
/// carries the session's baseline; without one, nothing is.
#[test]
fn a_pending_input_compares_with_the_baseline_its_trace_carries() {
    let sent = body("sys", &[user("a")]);
    let untraced = RequestTrace::default();
    assert!(PendingInput::begin(&untraced, "s", ENDPOINT, &sent).is_none());
    assert_eq!(untraced.input_prefix(), None);
    let baseline = InputBaseline::default();
    let first = RequestTrace::default();
    first.attach_input_baseline(baseline.clone());
    PendingInput::begin(&first, "s", ENDPOINT, &sent)
        .expect("a baseline")
        .accept();
    assert_eq!(
        first.input_prefix().map(|p| p.parts().previous_items),
        Some(None)
    );
    let second = RequestTrace::default();
    second.attach_input_baseline(baseline);
    let appended = body("sys", &[user("a"), user("b")]);
    let pending = PendingInput::begin(&second, "s", ENDPOINT, &appended).expect("a baseline");
    let parts = second.input_prefix().expect("recorded").parts();
    assert_eq!(
        (parts.previous_items, parts.first_changed_item),
        (Some(1), None)
    );
    drop(pending);
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
    let baseline = InputBaseline::default();
    observe(&baseline, "s", &[json!({"type": "custom", "v": 1})]);
    let observed = observe(&baseline, "s", &[json!({"type": "custom", "v": 2})]);
    assert_eq!(observed.first_changed_item, Some(0));
    assert_eq!(observed.first_changed_kind, None);
}

/// Nothing kept, measured or recorded carries content or the session key.
#[test]
fn nothing_kept_or_recorded_carries_content() {
    const SECRET: &str = "sk-proj-QX7hunter2SECRETtoken9d1f";
    let session = format!("cli:{SECRET}");
    let baseline = InputBaseline::default();
    observe(&baseline, &session, &[user(SECRET)]);
    let input = [
        user(&format!("{SECRET}!")),
        call(SECRET),
        output(SECRET, SECRET),
    ];
    let measured = MeasuredInput::of(&session, ENDPOINT, &body(SECRET, &input));
    let observed = compare(&baseline, &measured).expect("consistent");
    assert_eq!(observed.parts().first_changed_item, Some(0));
    let measured_text = format!("{measured:?} {observed:?} {baseline:?}");
    commit(&baseline, measured);
    let kept = baseline.read(|kept: Option<&AcceptedInput>| format!("{:?}", kept.expect("kept")));
    let recorded = serde_json::to_string(&observed).unwrap();
    for text in [measured_text, kept, recorded] {
        for fragment in ["sk-proj", "hunter2", "SECRET", "9d1f", "cli:"] {
            assert!(!text.contains(fragment), "{fragment} in {text}");
        }
    }
}
