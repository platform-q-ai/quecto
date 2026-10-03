//! #2423: an extension's `tool_result` carries images (`imageBlocks`),
//! admitted by `quecto_image`, and a tool sets its own timeout when it
//! registers (`timeoutSeconds`).
use std::time::Duration;

use quecto_image::samples::{encode, png};
use quecto_image::{ImageMime, ImageRefusal};
use serde_json::{Value, json};

use super::*;

fn tool_reg(name: &str, timeout_seconds: Option<Value>) -> ToolRegistration {
    ToolRegistration {
        name: name.into(),
        description: format!("{name} description"),
        parameters_schema: r#"{"type":"object"}"#.into(),
        stable_id: None,
        timeout_seconds,
    }
}

fn register(
    registry: &ClientToolRegistry,
    tools: &[ToolRegistration],
) -> (
    bool,
    AgentEvent,
    Vec<Arc<dyn crate::application::tools::ports::Tool>>,
) {
    handle_register_tools(RegisterToolsArgs {
        client_id: 1,
        id: Some("rt-1"),
        tools,
        registry,
        core_tool_names: &HashSet::new(),
    })
}

/// Park a call for `call_id` on client 1 and return its result receiver.
fn park(
    registry: &ClientToolRegistry,
    call_id: &str,
) -> tokio::sync::oneshot::Receiver<ToolResult> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    registry
        .lock()
        .unwrap()
        .entry(1)
        .or_default()
        .insert_pending(call_id.into(), "shot".into(), tx, Duration::from_secs(30));
    rx
}

fn deliver(registry: &ClientToolRegistry, call_id: &str, args: (&str, bool, Option<Value>)) {
    let (content, is_error, image_blocks) = args;
    handle_tool_result(ToolResultArgs {
        client_id: 1,
        tool_call_id: call_id,
        content,
        is_error,
        image_blocks,
        registry,
    });
}

/// The result one `tool_result` with `content` and `image_blocks` gives.
fn result_of(content: &str, image_blocks: Option<Value>) -> ToolResult {
    let registry = new_client_tool_registry();
    let mut rx = park(&registry, "call-1");
    deliver(&registry, "call-1", (content, false, image_blocks));
    rx.try_recv().expect("the call is resolved")
}

fn sent(result: &ToolResult) -> Vec<(String, String)> {
    result
        .image_blocks
        .iter()
        .map(|image| (image.mime_type().to_owned(), image.data().to_owned()))
        .collect()
}

#[test]
fn an_extensions_images_reach_the_tool_result() {
    let first = encode(&png(3, 2));
    let second = encode(&quecto_image::samples::sample(ImageMime::Webp));
    let blocks = json!([
        {"mimeType": "image/png", "data": first},
        {"mimeType": "image/webp", "data": second},
    ]);
    let result = result_of("two screenshots", Some(blocks));
    assert!(!result.is_error);
    assert_eq!(result.content, "two screenshots");
    assert_eq!(
        sent(&result),
        vec![
            ("image/png".to_owned(), first),
            ("image/webp".to_owned(), second)
        ]
    );
}

#[test]
fn no_image_blocks_or_null_is_todays_text_result() {
    for blocks in [None, Some(Value::Null), Some(json!([]))] {
        let result = result_of("22°C, sunny", blocks);
        assert_eq!(result.content, "22°C, sunny");
        assert!(!result.is_error);
        assert!(result.image_blocks.is_empty());
    }
}

#[test]
fn an_error_result_keeps_its_images() {
    let data = encode(&png(1, 1));
    let registry = new_client_tool_registry();
    let mut rx = park(&registry, "call-1");
    let blocks = json!([{"mimeType": "image/png", "data": data}]);
    deliver(
        &registry,
        "call-1",
        ("button not found", true, Some(blocks)),
    );
    let result = rx.try_recv().unwrap();
    assert!(result.is_error);
    assert_eq!(result.content, "button not found");
    assert_eq!(sent(&result), vec![("image/png".to_owned(), data)]);
}

#[test]
fn a_refused_image_makes_the_result_an_error_naming_it_and_keeps_the_text() {
    let good = encode(&png(1, 1));
    let blocks = json!([
        {"mimeType": "image/png", "data": good},
        {"mimeType": "image/png", "data": "not base64!"},
    ]);
    let result = result_of("shot", Some(blocks));
    assert!(result.is_error);
    assert!(result.image_blocks.is_empty(), "no image of a refused list");
    assert_eq!(
        result.content,
        "Error: imageBlocks[1]: data is not valid standard base64\n\nshot"
    );
}

#[test]
fn every_admission_refusal_is_named_exactly() {
    let oversized = "A".repeat(quecto_image::MAX_ENCODED_LEN + 4);
    let cases = [
        (
            json!({"mimeType": "image/svg+xml", "data": "PHN2Zz4="}),
            ImageRefusal::UnsupportedMime("image/svg+xml".into()),
        ),
        (
            json!({"mimeType": "image/png", "data": oversized}),
            ImageRefusal::TooLarge,
        ),
        (
            json!({"mimeType": "image/gif", "data": encode(&png(1, 1))}),
            ImageRefusal::SignatureMismatch(ImageMime::Gif),
        ),
        (
            json!({"mimeType": "image/png", "data": encode(b"\x89PNG\r\n\x1a\n")}),
            ImageRefusal::Unreadable(ImageMime::Png),
        ),
    ];
    for (block, refusal) in cases {
        let result = result_of("", Some(json!([block])));
        assert!(result.is_error);
        assert!(result.image_blocks.is_empty());
        assert!(
            result.content == format!("Error: imageBlocks[0]: {refusal}"),
            "{refusal}: got {:.200}",
            result.content
        );
    }
}

#[test]
fn malformed_image_blocks_are_named_exactly() {
    let one = encode(&png(1, 1));
    let too_many: Vec<Value> = (0..9)
        .map(|_| json!({"mimeType": "image/png", "data": one}))
        .collect();
    let cases = [
        (
            json!({"mimeType": "image/png"}),
            r#"Error: imageBlocks: expected an array of {"mimeType", "data"} objects"#,
        ),
        (
            json!([{"mimeType": "image/png"}]),
            r#"Error: imageBlocks[0]: expected an object with string "mimeType" and "data""#,
        ),
        (
            json!([{"mimeType": "image/png", "data": one}, null]),
            r#"Error: imageBlocks[1]: expected an object with string "mimeType" and "data""#,
        ),
        (
            json!(too_many),
            "Error: too many imageBlocks: 9; at most 8 per tool result",
        ),
    ];
    for (blocks, expected) in cases {
        let result = result_of("", Some(blocks));
        assert!(result.is_error);
        assert!(result.image_blocks.is_empty());
        assert_eq!(result.content, expected);
    }
}

#[test]
fn a_refused_result_leaves_the_extension_connected() {
    let registry = new_client_tool_registry();
    let mut first = park(&registry, "call-1");
    let mut second = park(&registry, "call-2");
    deliver(&registry, "call-1", ("", false, Some(json!("nope"))));
    assert!(first.try_recv().unwrap().is_error);
    assert!(registry.lock().unwrap().contains_key(&1), "still connected");
    deliver(&registry, "call-2", ("fine", false, None));
    assert_eq!(second.try_recv().unwrap().content, "fine");
}

#[test]
fn an_intercepted_tool_result_line_carries_its_images() {
    let data = encode(&png(1, 1));
    let line = json!({
        "type": "tool_result",
        "toolCallId": "call-1",
        "content": "shot",
        "imageBlocks": [{"mimeType": "image/png", "data": data}],
    })
    .to_string();
    let parsed = super::super::uds_tool_intercept::try_intercept_tool_result(&line)
        .expect("a tool_result line is intercepted");
    let registry = new_client_tool_registry();
    let mut rx = park(&registry, "call-1");
    deliver(
        &registry,
        &parsed.tool_call_id,
        (&parsed.content, parsed.is_error, parsed.image_blocks),
    );
    assert_eq!(
        sent(&rx.try_recv().unwrap()),
        vec![("image/png".to_owned(), data)]
    );
}

// ─── timeoutSeconds ───────────────────────────────────────────────────────

#[test]
fn a_timeout_outside_one_to_six_hundred_seconds_is_refused_exactly() {
    let cases = [
        (json!(0), "0"),
        (json!(601), "601"),
        (json!(-1), "-1"),
        (json!(1.5), "1.5"),
        (json!("30"), r#""30""#),
    ];
    for (value, echoed) in cases {
        let registry = new_client_tool_registry();
        let tools = [tool_reg("fine", None), tool_reg("shot", Some(value))];
        let (ok, event, tools) = register(&registry, &tools);
        assert!(!ok);
        assert!(tools.is_empty(), "nothing of the batch is registered");
        assert!(registry.lock().unwrap().is_empty());
        let expected = format!(
            "tool 'shot': timeoutSeconds must be a whole number of seconds from 1 to 600, got {echoed}"
        );
        let line: Value = serde_json::from_str(&event.to_json_line()).unwrap();
        assert_eq!(line["error"], expected.as_str());
        assert_eq!(line["success"], false);
    }
}

#[test]
fn a_timeout_within_the_range_or_null_is_accepted() {
    for value in [json!(1), json!(600), Value::Null] {
        let registry = new_client_tool_registry();
        let (ok, _, tools) = register(&registry, &[tool_reg("shot", Some(value))]);
        assert!(ok);
        assert_eq!(tools.len(), 1);
    }
}

async fn timed_out_message(timeout_seconds: Option<Value>) -> String {
    let registry = new_client_tool_registry();
    let (ok, _, tools) = register(&registry, &[tool_reg("shot", timeout_seconds)]);
    assert!(ok);
    // Nobody answers: the call waits out the tool's timeout.
    let result = tools[0].execute("{}").await.unwrap();
    assert!(result.is_error);
    result.content
}

#[tokio::test(start_paused = true)]
async fn a_registered_timeout_replaces_the_default_for_that_tool() {
    assert_eq!(
        timed_out_message(Some(json!(300))).await,
        "Extension timed out after 300s executing tool 'shot'"
    );
    assert_eq!(
        timed_out_message(None).await,
        "Extension timed out after 30s executing tool 'shot'"
    );
}

#[tokio::test]
async fn a_forwarded_calls_pending_slot_lives_as_long_as_its_timeout() {
    let registry = new_client_tool_registry();
    let (writer_tx, mut writer_rx) = tokio::sync::mpsc::channel::<String>(1);
    register_client_writer(&registry, 1, writer_tx.clone());
    let (ok, _, tools) = register(&registry, &[tool_reg("shot", Some(json!(300)))]);
    assert!(ok);
    let mut requests = registry
        .lock()
        .unwrap()
        .get_mut(&1)
        .unwrap()
        .tool_request_rxs
        .remove("shot")
        .unwrap();
    let tool = tools[0].clone();
    let call = tokio::spawn(async move { tool.execute("{}").await });
    let request = requests.recv().await.unwrap();
    handle_one_request("shot", request, 1, &registry, &Some(writer_tx)).await;
    writer_rx.recv().await.expect("execute_tool sent");
    let deadline = registry.lock().unwrap()[&1]
        .pending_results
        .values()
        .next()
        .expect("the call is pending")
        .deadline;
    assert!(
        deadline > std::time::Instant::now() + Duration::from_secs(200),
        "the slot outlives a 30 s default"
    );
    call.abort();
}
