//! #2423: MCP `image` content items reach quecto as `imageBlocks`, admitted
//! by `quecto_image`; an item quecto cannot send is a text marker instead.
use super::*;
use quecto_image::samples::{encode, png, png_with_body};
use quecto_image::{ImageMime, ImageRefusal, MAX_ENCODED_LEN};
use serde_json::json;

fn image_item(mime: &str, data: &str) -> Value {
    json!({"type": "image", "mimeType": mime, "data": data})
}

fn text_item(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

fn result_of(items: Vec<Value>) -> McpToolResult {
    mcp_tool_result(json!({ "content": items }))
}

fn marker(refusal: ImageRefusal) -> String {
    format!("[image omitted: {refusal}]")
}

#[test]
fn text_only_results_are_joined_as_before() {
    let result = result_of(vec![text_item("one"), text_item("two")]);
    assert_eq!(result.content, "one\ntwo");
    assert!(result.image_blocks.is_empty());
}

#[test]
fn a_result_with_neither_text_nor_images_is_its_json_as_before() {
    let raw = json!({"content": [{"type": "resource", "resource": {"uri": "x://y"}}]});
    let result = mcp_tool_result(raw.clone());
    assert_eq!(result.content, serde_json::to_string_pretty(&raw).unwrap());
    assert!(result.image_blocks.is_empty());
}

#[test]
fn mixed_text_and_images_keep_the_text_and_carry_the_images_in_order() {
    let first = encode(&png(4, 3));
    let second = encode(&quecto_image::samples::jpeg(2, 2));
    let result = result_of(vec![
        text_item("before"),
        image_item("image/png", &first),
        text_item("after"),
        image_item("image/jpeg", &second),
    ]);
    assert_eq!(result.content, "before\nafter");
    let sent: Vec<(ImageMime, &str)> = result
        .image_blocks
        .iter()
        .map(|image| (image.mime(), image.data()))
        .collect();
    assert_eq!(
        sent,
        vec![
            (ImageMime::Png, first.as_str()),
            (ImageMime::Jpeg, second.as_str())
        ]
    );
}

#[test]
fn an_image_only_result_carries_the_image_and_no_text() {
    let data = encode(&png(1, 1));
    let result = result_of(vec![image_item("image/png", &data)]);
    assert_eq!(result.content, "");
    assert_eq!(result.image_blocks.len(), 1);
    assert_eq!(result.image_blocks[0].data(), data);
}

#[test]
fn an_unsupported_mime_type_is_a_marker_in_the_images_place() {
    let result = result_of(vec![
        text_item("drawing"),
        image_item("image/svg+xml", "PHN2Zz4="),
        text_item("done"),
    ]);
    let expected = marker(ImageRefusal::UnsupportedMime("image/svg+xml".into()));
    assert_eq!(result.content, format!("drawing\n{expected}\ndone"));
    assert!(result.image_blocks.is_empty());
}

#[test]
fn an_image_admission_refuses_is_a_marker_naming_the_refusal() {
    let oversized = "A".repeat(MAX_ENCODED_LEN + 4);
    let bare_signature = encode(b"\x89PNG\r\n\x1a\n");
    let result = result_of(vec![
        image_item("image/png", &oversized),
        image_item("image/png", "not base64!"),
        image_item("image/gif", &encode(&png(1, 1))),
        image_item("image/png", &bare_signature),
    ]);
    let expected = [
        marker(ImageRefusal::TooLarge),
        marker(ImageRefusal::InvalidBase64),
        marker(ImageRefusal::SignatureMismatch(ImageMime::Gif)),
        marker(ImageRefusal::Unreadable(ImageMime::Png)),
    ]
    .join("\n");
    assert_eq!(result.content, expected);
    assert!(result.image_blocks.is_empty());
}

#[test]
fn an_image_item_without_string_fields_is_a_marker() {
    let result = result_of(vec![
        json!({"type": "image", "mimeType": "image/png"}),
        json!({"type": "image", "mimeType": 7, "data": "AAAA"}),
    ]);
    let expected = format!("{MALFORMED_IMAGE_MARKER}\n{MALFORMED_IMAGE_MARKER}");
    assert_eq!(result.content, expected);
    assert!(result.image_blocks.is_empty());
}

#[test]
fn images_past_the_per_result_limit_are_markers() {
    let data = encode(&png(1, 1));
    let items = (0..quecto_image::MAX_IMAGES_PER_MESSAGE + 2)
        .map(|_| image_item("image/png", &data))
        .collect();
    let result = result_of(items);
    assert_eq!(
        result.image_blocks.len(),
        quecto_image::MAX_IMAGES_PER_MESSAGE
    );
    assert_eq!(
        result.content,
        format!("{TOO_MANY_IMAGES_MARKER}\n{TOO_MANY_IMAGES_MARKER}")
    );
}

#[test]
fn images_that_would_pass_the_uds_line_limit_are_markers() {
    // Three images of 3.5 MB: their base64 (about 4.7 MiB each) fits the
    // 8 MiB line once, not twice.
    let big = |seed| encode(&png_with_body(seed, 1, 3_500_000));
    let result = result_of(vec![
        text_item("three screenshots"),
        image_item("image/png", &big(1)),
        image_item("image/png", &big(2)),
        image_item("image/png", &big(3)),
    ]);
    assert_eq!(result.image_blocks.len(), 1, "only the first fits");
    assert_eq!(
        result.content,
        format!("three screenshots\n{LINE_LIMIT_MARKER}\n{LINE_LIMIT_MARKER}")
    );
    let line = tool_result_line("uds-1", &result);
    assert!(line.len() <= quecto_line_io::PROTOCOL_LINE_CAP_BYTES);
}

#[test]
fn the_tool_result_line_carries_image_blocks_only_when_there_are_some() {
    let data = encode(&png(1, 1));
    let with = result_of(vec![text_item("t"), image_item("image/png", &data)]);
    let value: Value = serde_json::from_str(&tool_result_line("uds-1", &with)).unwrap();
    assert_eq!(
        value,
        json!({
            "type": "tool_result",
            "toolCallId": "uds-1",
            "content": "t",
            "isError": false,
            "imageBlocks": [{"mimeType": "image/png", "data": data}]
        })
    );
    let without = McpToolResult::error("t");
    let value: Value = serde_json::from_str(&tool_result_line("uds-1", &without)).unwrap();
    assert_eq!(
        value,
        json!({"type": "tool_result", "toolCallId": "uds-1", "content": "t", "isError": true})
    );
}

#[test]
fn a_text_too_long_for_the_line_is_an_error_result() {
    let huge = McpToolResult::text("\"".repeat(quecto_line_io::PROTOCOL_LINE_CAP_BYTES / 2));
    let value: Value = serde_json::from_str(&tool_result_line("uds-9", &huge)).unwrap();
    assert_eq!(value["isError"], true);
    assert_eq!(value["toolCallId"], "uds-9");
    let content = value["content"].as_str().unwrap();
    assert!(
        content.starts_with("MCP result too large for the Quecto UDS line limit"),
        "{content}"
    );
}

/// Review low: an MCP tool error (`result.isError`) is an error result,
/// with its text and images as for any result.
#[test]
fn an_mcp_tool_error_is_an_error_tool_result() {
    let failed = mcp_tool_result(json!({"content": [text_item("no such page")], "isError": true}));
    assert!(failed.is_error);
    assert_eq!(failed.content, "no such page");
    let line: Value = serde_json::from_str(&tool_result_line("uds-1", &failed)).unwrap();
    assert_eq!(line["isError"], true);
    for flag in [json!(false), json!("true"), Value::Null] {
        let result = mcp_tool_result(json!({"content": [text_item("ok")], "isError": flag}));
        assert!(!result.is_error, "only a JSON true is an error");
    }
}

/// End to end: an MCP server's image reaches quecto's UDS socket as an
/// `imageBlocks` entry of the `tool_result`.
#[tokio::test]
async fn an_mcp_image_reaches_the_uds_tool_result() {
    use crate::{McpClient, McpTool, RegisteredMcpTools, build_mapping, build_registrations};
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use wiremock::matchers::{body_partial_json, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let data = encode(&png(2, 2));
    let mcp_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"method": "tools/call"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": "call",
            "result": {"content": [text_item("the screen"), image_item("image/png", &data)]}
        })))
        .mount(&mcp_server)
        .await;

    let temp = tempfile::tempdir().unwrap();
    let socket = temp.path().join("agent.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let expected = data.clone();
    let server_task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut reader = BufReader::new(&mut stream);
        let mut register_line = String::new();
        reader.read_line(&mut register_line).await.unwrap();
        drop(reader);
        let response = json!({"type":"response","id":"quecto-mcp-register","command":"register_tools","success":true});
        let execute = json!({"type":"execute_tool","toolCallId":"uds-1","toolName":"browser_screenshot","arguments":"{}"});
        for event in [response, execute] {
            stream
                .write_all(format!("{event}\n").as_bytes())
                .await
                .unwrap();
        }
        let mut reader = BufReader::new(&mut stream);
        let mut result_line = String::new();
        reader.read_line(&mut result_line).await.unwrap();
        let result: Value = serde_json::from_str(result_line.trim()).unwrap();
        assert_eq!(result["content"], "the screen");
        assert_eq!(result["isError"], false);
        assert_eq!(
            result["imageBlocks"],
            json!([{"mimeType": "image/png", "data": expected}])
        );
    });
    let tools = vec![McpTool {
        name: "browser.screenshot".into(),
        description: "".into(),
        input_schema: json!({}),
    }];
    crate::serve_uds_extension(
        &socket,
        RegisteredMcpTools {
            registrations: build_registrations(&tools).unwrap(),
            mapping: build_mapping(&tools).unwrap(),
        },
        McpClient::new(mcp_server.uri(), "token".into()),
        Duration::from_secs(1),
    )
    .await
    .unwrap();
    server_task.await.unwrap();
}

/// Review low: a line is measured without rendering it; the measure is
/// exactly the rendering's, escapes included.
#[test]
fn a_line_is_measured_exactly_as_it_is_rendered() {
    let data = encode(&png(1, 1));
    let texts = [
        "",
        "plain",
        "quote \" slash \\ tab \t nl \n",
        "\u{1} bell \u{7f} é ✓",
    ];
    for text in texts {
        for images in [0, 1, 3] {
            for is_error in [false, true] {
                let result = McpToolResult {
                    content: text.to_owned(),
                    image_blocks: (0..images).map(|_| mcp_image(&data)).collect(),
                    is_error,
                };
                let line = tool_result_line("uds-\"7\"", &result);
                let measured = line_len(
                    "uds-\"7\"",
                    (json_str_len(text), is_error),
                    &result.image_blocks,
                );
                assert_eq!(line.len(), measured, "{text:?} {images} {is_error}");
            }
        }
    }
}

fn mcp_image(data: &str) -> ImageAttachment {
    ImageAttachment::new(quecto_image::ImagePayload::new("image/png", data)).unwrap()
}

/// Review round 2 M1: a `result` that is not an object (a string, an
/// array) is its JSON, as before; it never panics the bridge.
#[test]
fn a_result_that_is_not_an_object_is_its_json() {
    for raw in [json!("done"), json!([1, "two"]), json!(7), Value::Null] {
        let result = mcp_tool_result(raw.clone());
        assert_eq!(result.content, serde_json::to_string_pretty(&raw).unwrap());
        assert!(result.image_blocks.is_empty() && !result.is_error);
    }
}

/// Review round 2 L1: a `content` that is not a list is kept as it came.
#[test]
fn a_content_that_is_not_a_list_is_kept_as_it_came() {
    let raw = json!({"content": "hello", "isError": false});
    let result = mcp_tool_result(raw.clone());
    assert_eq!(result.content, serde_json::to_string_pretty(&raw).unwrap());
}

/// Review round 2 L1: with images, another item (a resource, a link) is
/// never dropped silently: it is its JSON, in its place.
#[test]
fn with_images_another_item_is_its_json_in_its_place() {
    let data = encode(&png(1, 1));
    let resource = json!({"type": "resource", "resource": {"uri": "file:///a.txt", "text": "A"}});
    let link = json!({"type": "resource_link", "uri": "https://e.example/b", "name": "b"});
    let result = result_of(vec![
        text_item("before"),
        resource.clone(),
        image_item("image/png", &data),
        link.clone(),
    ]);
    assert_eq!(result.image_blocks.len(), 1);
    assert_eq!(result.content, format!("before\n{resource}\n{link}"));
}

/// Review round 2 L1: an item too large to show is a marker naming it.
#[test]
fn with_images_a_large_other_item_is_a_marker_naming_it() {
    let data = encode(&png(1, 1));
    let blob = json!({"type": "resource", "resource": {"uri": "file:///big.bin", "blob": "A".repeat(100_000)}});
    let size = blob.to_string().len();
    let result = result_of(vec![blob, image_item("image/png", &data)]);
    assert_eq!(
        result.content,
        format!("[content omitted: a \"resource\" item of {size} bytes]")
    );
}
