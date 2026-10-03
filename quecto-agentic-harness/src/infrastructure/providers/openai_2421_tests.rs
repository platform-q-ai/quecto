//! #2421: images on the chat-completions wire. A user message's images are
//! `image_url` content parts. A `tool` message carries text only (the API
//! refuses images there): after the last tool message of a call batch, one
//! `user` message carries the batch's images, each labelled with its call.
//! A message with no images goes exactly as before.

use super::OpenAiProvider;
use crate::application::providers::ports::ChatRequest;
use crate::domain::message::{Message, ToolCall, UserImageBlock};
use crate::domain::tool::ImageBlock;

fn body(messages: &[Message]) -> serde_json::Value {
    let request = ChatRequest {
        trace: None,
        admission: None,
        messages,
        tools: &[],
        model: "gpt-5.5",
        max_tokens: 256,
        temperature: 0.2,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    OpenAiProvider::build_chat_completions_body_for_test("openai", &request)["messages"].clone()
}

fn calls(ids: &[(&str, &str)]) -> Message {
    let mut assistant = Message::assistant("", vec![]);
    assistant.tool_calls = ids
        .iter()
        .map(|(id, name)| ToolCall {
            id: id.to_string(),
            name: name.to_string(),
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

fn tool_with_images(id: &str, name: &str, text: &str, images: &[(&'static str, &str)]) -> Message {
    let mut message = Message::tool(id, text);
    message.tool_name = Some(name.into());
    message.image_blocks = images
        .iter()
        .map(|(mime, data)| ImageBlock {
            mime_type: mime,
            data: data.to_string(),
        })
        .collect();
    message
}

fn image_part(url: &str) -> serde_json::Value {
    // Review D: every image goes at `"detail": "high"`.
    serde_json::json!({"type": "image_url", "image_url": {"url": url, "detail": "high"}})
}

#[test]
fn a_user_message_with_images_is_text_then_each_image() {
    let messages = vec![user_with_images(
        "compare these",
        &[("image/png", "cG5n"), ("image/jpeg", "anBn")],
    )];
    assert_eq!(
        body(&messages),
        serde_json::json!([{
            "role": "user",
            "content": [
                {"type": "text", "text": "compare these"},
                image_part("data:image/png;base64,cG5n"),
                image_part("data:image/jpeg;base64,anBn"),
            ],
        }])
    );
}

#[test]
fn a_user_message_with_images_and_no_text_has_no_text_part() {
    let messages = vec![user_with_images("", &[("image/webp", "d2Vi")])];
    assert_eq!(
        body(&messages)[0]["content"],
        serde_json::json!([image_part("data:image/webp;base64,d2Vi")])
    );
}

#[test]
fn a_batchs_images_follow_its_last_tool_message_in_one_user_message() {
    let messages = vec![
        Message::user("look"),
        calls(&[("call_1", "read"), ("call_2", "bash"), ("call_3", "shot")]),
        tool_with_images(
            "call_1",
            "read",
            "Read image file a.png",
            &[("image/png", "cG5n")],
        ),
        Message::tool("call_2", "ok"),
        tool_with_images(
            "call_3",
            "shot",
            "",
            &[("image/gif", "Z2lm"), ("image/jpeg", "anBn")],
        ),
        Message::assistant("Seen.", vec![]),
    ];
    let sent = body(&messages);
    let roles: Vec<&str> = sent
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(
        roles,
        [
            "user",
            "assistant",
            "tool",
            "tool",
            "tool",
            "user",
            "assistant"
        ],
        "the tool results stay adjacent to their calls"
    );
    assert_eq!(sent[2]["content"], "Read image file a.png");
    assert_eq!(sent[4]["content"], "", "a tool message carries text only");
    assert_eq!(
        sent[5],
        serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "Image from tool call call_1 (read):"},
                image_part("data:image/png;base64,cG5n"),
                {"type": "text", "text": "Image from tool call call_3 (shot):"},
                image_part("data:image/gif;base64,Z2lm"),
                {"type": "text", "text": "Image from tool call call_3 (shot):"},
                image_part("data:image/jpeg;base64,anBn"),
            ],
        })
    );
}

#[test]
fn a_batch_at_the_end_of_the_conversation_is_followed_by_its_images() {
    let messages = vec![
        Message::user("look"),
        calls(&[("call_1", "read")]),
        tool_with_images("call_1", "read", "img", &[("image/png", "cG5n")]),
    ];
    let sent = body(&messages);
    assert_eq!(sent.as_array().unwrap().len(), 4);
    assert_eq!(sent[3]["role"], "user");
    assert_eq!(
        sent[3]["content"][1],
        image_part("data:image/png;base64,cG5n")
    );
}

#[test]
fn a_batch_without_images_has_no_following_user_message() {
    let messages = vec![
        Message::user("look"),
        calls(&[("call_1", "read")]),
        Message::tool("call_1", "text"),
        Message::assistant("Done.", vec![]),
    ];
    assert_eq!(body(&messages).as_array().unwrap().len(), 4);
}

/// The bytes a text-only conversation became before #2421.
#[test]
fn a_conversation_without_images_serializes_byte_for_byte_as_before() {
    let messages = vec![
        Message::system("Be brief."),
        Message::user("Hi"),
        calls(&[("call_1", "read")]),
        Message::tool("call_1", "file"),
        Message::assistant("Done.", vec![]),
    ];
    assert_eq!(
        serde_json::to_string(&body(&messages)).unwrap(),
        concat!(
            r#"[{"role":"system","content":"Be brief."},"#,
            r#"{"role":"user","content":"Hi"},"#,
            r#"{"role":"assistant","content":"","tool_calls":[{"id":"call_1","type":"function","function":{"name":"read","arguments":"{\"path\":\"a.png\"}"}}]},"#,
            r#"{"role":"tool","content":"file","tool_call_id":"call_1"},"#,
            r#"{"role":"assistant","content":"Done."}]"#,
        )
    );
}

/// A long history with images in prompts and results: every message an
/// earlier request sent is sent again unchanged.
#[test]
fn every_earlier_message_is_unchanged_as_the_conversation_grows() {
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
        history.push(calls(&[(id.as_str(), "read")]));
        match turn % 2 {
            0 => history.push(Message::tool(&id, format!("text {turn}"))),
            _ => history.push(tool_with_images(
                &id,
                "read",
                "img",
                &[("image/png", "cG5n")],
            )),
        }
        history.push(Message::assistant(format!("answer {turn}"), vec![]));

        let sent = body(&history);
        let sent = sent.as_array().unwrap();
        assert!(sent.len() > sent_before.len());
        for (index, message) in sent_before.iter().enumerate() {
            assert_eq!(
                &sent[index], message,
                "message {index} changed at turn {turn}"
            );
        }
        sent_before = sent.clone();
    }
}

/// #2421 review N2: a request that ends on a tool batch with images (the
/// trailing image message) is sent unchanged at the head of the next one.
#[test]
fn a_request_ending_on_an_image_batch_is_the_next_requests_prefix() {
    let mut history = vec![
        Message::system("Be brief."),
        Message::user("look"),
        calls(&[("call_1", "read"), ("call_2", "shot")]),
        tool_with_images("call_1", "read", "img", &[("image/png", "cG5n")]),
        tool_with_images("call_2", "shot", "", &[("image/jpeg", "anBn")]),
    ];
    let before = body(&history);
    let before = before.as_array().unwrap();
    assert_eq!(before.last().unwrap()["role"], "user", "ends on the images");
    history.push(Message::assistant("Seen both.", vec![]));
    history.push(Message::user("next"));
    let after = body(&history);
    let after = after.as_array().unwrap();
    assert_eq!(&after[..before.len()], &before[..]);
    assert_eq!(after.len(), before.len() + 2);
}
