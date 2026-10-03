//! #2421: images on the Responses wire. A user message with images is a
//! content array of `input_text` and `input_image` parts (always with
//! `"detail": "high"`, review D); a tool result with
//! images is a `function_call_output` whose `output` is such an array. A
//! message with no images goes exactly as before, so a conversation's cache
//! prefix never changes.
use super::*;
use crate::domain::message::{ToolCall, UserImageBlock};
use crate::domain::tool::ImageBlock;

fn call(id: &str, name: &str) -> Message {
    let mut assistant = Message::assistant("", vec![]);
    assistant.tool_calls = vec![ToolCall {
        id: id.to_string(),
        name: name.into(),
        arguments: r#"{"path":"a.png"}"#.to_string(),
    }];
    assistant
}

fn calls(ids: &[&str]) -> Message {
    let mut assistant = Message::assistant("", vec![]);
    assistant.tool_calls = ids
        .iter()
        .map(|id| ToolCall {
            id: id.to_string(),
            name: "read".into(),
            arguments: r#"{"path":"a.png"}"#.to_string(),
        })
        .collect();
    assistant
}

fn user_with_images(text: &str, images: &[(&str, &str)]) -> Message {
    let mut message = Message::user(text);
    message.user_image_blocks = images
        .iter()
        .map(|(mime, data)| {
            UserImageBlock::unchecked_for_tests(
                quecto_image::ImageMime::parse_exact(mime).expect("an admitted type"),
                data.to_string(),
            )
        })
        .collect();
    message
}

fn tool_with_images(id: &str, text: &str, images: &[(&'static str, &str)]) -> Message {
    let mut message = Message::tool(id, text);
    message.tool_name = Some("read".into());
    message.image_blocks = images
        .iter()
        .map(|(mime, data)| {
            ImageBlock::unchecked_for_tests(
                quecto_image::ImageMime::parse_exact(mime).expect("an admitted type"),
                data.to_string(),
            )
        })
        .collect();
    message
}

#[test]
fn a_user_message_with_images_is_text_then_each_image() {
    let messages = vec![user_with_images(
        "compare these",
        &[("image/png", "cG5n"), ("image/jpeg", "anBn")],
    )];
    let (_, input) = CodexProvider::build_input(&messages);
    assert_eq!(
        input,
        vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "input_text", "text": "compare these"},
                {"type": "input_image", "image_url": "data:image/png;base64,cG5n", "detail": "high"},
                {"type": "input_image", "image_url": "data:image/jpeg;base64,anBn", "detail": "high"},
            ],
        })]
    );
}

#[test]
fn a_user_message_with_images_and_no_text_has_no_text_part() {
    let messages = vec![user_with_images("", &[("image/webp", "d2Vi")])];
    let (_, input) = CodexProvider::build_input(&messages);
    assert_eq!(
        input[0]["content"],
        serde_json::json!([
            {"type": "input_image", "image_url": "data:image/webp;base64,d2Vi", "detail": "high"},
        ])
    );
}

#[test]
fn a_tool_result_with_images_is_an_output_array() {
    let messages = vec![
        Message::user("look"),
        call("call_1", "read"),
        tool_with_images("call_1", "Read image file a.png", &[("image/png", "cG5n")]),
    ];
    let (_, input) = CodexProvider::build_input(&messages);
    assert_eq!(
        input[2],
        serde_json::json!({
            "type": "function_call_output",
            "call_id": "call_1",
            "output": [
                {"type": "input_text", "text": "Read image file a.png"},
                {"type": "input_image", "image_url": "data:image/png;base64,cG5n", "detail": "high"},
            ],
        })
    );
}

#[test]
fn each_result_of_one_batch_carries_its_own_images() {
    let messages = vec![
        Message::user("look"),
        calls(&["call_1", "call_2", "call_3"]),
        tool_with_images("call_1", "one", &[("image/png", "b25l")]),
        Message::tool("call_2", "two"),
        tool_with_images(
            "call_3",
            "",
            &[("image/gif", "Z2lm"), ("image/jpeg", "anBn")],
        ),
    ];
    let (_, input) = CodexProvider::build_input(&messages);
    let outputs: Vec<&serde_json::Value> = input
        .iter()
        .filter(|item| item["type"] == "function_call_output")
        .map(|item| &item["output"])
        .collect();
    assert_eq!(
        outputs,
        vec![
            &serde_json::json!([
                {"type": "input_text", "text": "one"},
                {"type": "input_image", "image_url": "data:image/png;base64,b25l", "detail": "high"},
            ]),
            &serde_json::json!("two"),
            &serde_json::json!([
                {"type": "input_image", "image_url": "data:image/gif;base64,Z2lm", "detail": "high"},
                {"type": "input_image", "image_url": "data:image/jpeg;base64,anBn", "detail": "high"},
            ]),
        ]
    );
}

/// The bytes a text-only conversation became before #2421: every item, as
/// sent. Any change here moves the cache prefix of existing sessions.
#[test]
fn a_conversation_without_images_serializes_byte_for_byte_as_before() {
    let messages = vec![
        Message::system("Be brief."),
        Message::user("Hi"),
        call("call_1", "read"),
        Message::tool("call_1", "file"),
        Message::assistant("Done.", vec![]),
    ];
    let (instructions, input) = CodexProvider::build_input(&messages);
    assert_eq!(instructions.as_deref(), Some("Be brief."));
    assert_eq!(
        serde_json::to_string(&input).unwrap(),
        concat!(
            r#"[{"role":"user","content":"Hi"},"#,
            r#"{"type":"function_call","call_id":"call_1","name":"read","arguments":"{\"path\":\"a.png\"}"},"#,
            r#"{"type":"function_call_output","call_id":"call_1","output":"file"},"#,
            r#"{"role":"assistant","phase":"final_answer","content":"Done."}]"#,
        )
    );
}

/// A long history, text-only and then with images: every item an earlier
/// request sent is sent again unchanged, whatever came after it.
#[test]
fn every_earlier_item_is_unchanged_as_the_conversation_grows() {
    let mut history = vec![Message::system("Be brief.")];
    let mut sent_before: Vec<serde_json::Value> = Vec::new();
    for turn in 0..40 {
        let id = format!("call_{turn}");
        match turn % 3 {
            0 => history.push(Message::user(format!("question {turn}"))),
            1 => history.push(user_with_images(
                &format!("look {turn}"),
                &[("image/png", "cG5n")],
            )),
            _ => history.push(user_with_images("", &[("image/jpeg", "anBn")])),
        }
        history.push(call(&id, "read"));
        match turn % 2 {
            0 => history.push(Message::tool(&id, format!("text {turn}"))),
            _ => history.push(tool_with_images(&id, "img", &[("image/png", "cG5n")])),
        }
        history.push(Message::assistant(format!("answer {turn}"), vec![]));

        let (_, input) = CodexProvider::build_input(&history);
        assert!(input.len() > sent_before.len());
        for (index, item) in sent_before.iter().enumerate() {
            assert_eq!(&input[index], item, "item {index} changed at turn {turn}");
        }
        sent_before = input;
    }
}

/// #2421 review N2: a request that ends on a tool batch with images is sent
/// unchanged at the head of the next one.
#[test]
fn a_request_ending_on_an_image_batch_is_the_next_requests_prefix() {
    let mut history = vec![
        Message::system("Be brief."),
        Message::user("look"),
        calls(&["call_1", "call_2"]),
        tool_with_images("call_1", "img", &[("image/png", "cG5n")]),
        tool_with_images("call_2", "", &[("image/jpeg", "anBn")]),
    ];
    let (_, before) = CodexProvider::build_input(&history);
    history.push(Message::assistant("Seen both.", vec![]));
    history.push(Message::user("next"));
    let (_, after) = CodexProvider::build_input(&history);
    assert_eq!(&after[..before.len()], &before[..]);
    assert_eq!(after.len(), before.len() + 2);
}
