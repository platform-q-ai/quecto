//! #2421: an image goes to a model only when the model declares image input;
//! otherwise each image is a marker in the message it belonged to.
use super::*;
use crate::domain::message::UserImageBlock;
use crate::domain::tool::ImageBlock;

const MODEL: &str = "fireworks/glm-5p2";

fn modalities(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn user_with_images(text: &str, count: usize) -> Message {
    let mut message = Message::user(text);
    message.user_image_blocks = (0..count)
        .map(|index| UserImageBlock {
            mime_type: "image/png".into(),
            data: format!("cG5n{index}"),
        })
        .collect();
    message
}

fn tool_with_image(id: &str, text: &str) -> Message {
    let mut message = Message::tool(id, text);
    message.tool_name = Some("read".into());
    message.image_blocks = vec![ImageBlock {
        mime_type: "image/jpeg",
        data: "anBn".into(),
    }];
    message
}

#[test]
fn a_model_that_declares_image_input_takes_images() {
    assert!(takes_images(&modalities(&["text", "image"])));
    assert!(takes_images(&modalities(&["image"])));
}

#[test]
fn a_model_that_declares_no_image_input_takes_none() {
    assert!(!takes_images(&modalities(&["text"])));
    assert!(!takes_images(&modalities(&["text", "audio"])));
    assert!(
        !takes_images(&[]),
        "a model that declares nothing takes none"
    );
}

#[test]
fn the_marker_names_the_model() {
    assert_eq!(
        not_sent_marker(MODEL),
        "[image not sent: fireworks/glm-5p2 takes no image input]"
    );
}

#[test]
fn a_model_that_takes_images_is_sent_the_conversation_as_it_is() {
    let messages = vec![user_with_images("look", 1), tool_with_image("c1", "read")];
    let sent = for_model(&messages, MODEL, true);
    assert!(matches!(sent, Cow::Borrowed(_)), "nothing is copied");
    assert_eq!(sent[0].user_image_blocks.len(), 1);
    assert_eq!(sent[1].image_blocks.len(), 1);
}

#[test]
fn a_conversation_without_images_is_never_copied() {
    let messages = vec![Message::user("hello"), Message::assistant("hi", vec![])];
    assert!(matches!(
        for_model(&messages, MODEL, false),
        Cow::Borrowed(_)
    ));
}

#[test]
fn each_user_image_becomes_a_marker_after_the_text() {
    let messages = vec![user_with_images("compare these", 2)];
    let sent = for_model(&messages, MODEL, false);
    let marker = not_sent_marker(MODEL);
    assert_eq!(
        sent[0].content,
        format!("compare these\n{marker}\n{marker}")
    );
    assert!(sent[0].user_image_blocks.is_empty(), "no image is sent");
    assert_eq!(
        messages[0].user_image_blocks.len(),
        2,
        "the conversation itself keeps its images"
    );
}

#[test]
fn an_image_with_no_text_becomes_the_marker_alone() {
    let messages = vec![user_with_images("", 1)];
    let sent = for_model(&messages, MODEL, false);
    assert_eq!(sent[0].content, not_sent_marker(MODEL));
}

#[test]
fn a_tool_results_image_becomes_a_marker_in_that_result() {
    let messages = vec![
        Message::user("read it"),
        tool_with_image("c1", "Read image file a.jpg"),
        Message::tool("c2", "plain"),
    ];
    let sent = for_model(&messages, MODEL, false);
    assert_eq!(
        sent[1].content,
        format!("Read image file a.jpg\n{}", not_sent_marker(MODEL))
    );
    assert!(sent[1].image_blocks.is_empty());
    assert_eq!(sent[1].tool_call_id.as_deref(), Some("c1"));
    assert_eq!(sent[1].id(), messages[1].id(), "the same message, marked");
    assert_eq!(sent[0].content, "read it");
    assert_eq!(sent[2].content, "plain");
}

#[test]
fn a_message_is_marked_the_same_whatever_follows_it() {
    let earlier = vec![user_with_images("look", 1)];
    let mut later = earlier.clone();
    later.push(Message::assistant("seen", vec![]));
    later.push(tool_with_image("c1", "more"));
    assert_eq!(
        for_model(&earlier, MODEL, false)[0].content,
        for_model(&later, MODEL, false)[0].content
    );
}
