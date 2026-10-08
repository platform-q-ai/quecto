use super::*;

// --- build_user_content ---

#[test]
fn plain_text_returns_string() {
    let m = Message::user("hello world");
    let content = build_user_content(&m);
    assert_eq!(
        content,
        Some(serde_json::Value::String("hello world".to_string()))
    );
}

#[test]
fn empty_content_returns_none() {
    let m = Message::user("");
    assert_eq!(build_user_content(&m), None);
}

#[test]
fn whitespace_only_returns_none() {
    let m = Message::user("   \n\t  ");
    assert_eq!(build_user_content(&m), None);
}

#[test]
fn text_with_images_vision_returns_array() {
    let mut m = Message::user("describe this");
    m.user_image_blocks.push(
        crate::domain::conversation::value_objects::message::UserImageBlock::unchecked_for_tests(
            quecto_image::ImageMime::Png,
            "base64data",
        ),
    );
    let content = build_user_content(&m);
    assert!(content.is_some());
    let arr = content.unwrap();
    assert!(arr.is_array());
    let blocks = arr.as_array().unwrap();
    assert_eq!(blocks.len(), 2); // text + image
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[1]["type"], "image");
}

/// A block carries its admitted type (#2422), the allowlist Anthropic
/// takes (`ImageMime::parse_exact`): each is sent with its wire spelling.
#[test]
fn every_admitted_type_is_sent_with_its_wire_spelling() {
    for mime in quecto_image::ImageMime::ALL {
        let block =
            crate::domain::conversation::value_objects::message::UserImageBlock::sample(mime);
        let data = block.data().to_owned();
        let m = Message::user("describe").with_user_images(vec![block]);
        let content = build_user_content(&m).expect("sent");
        assert_eq!(content[1]["source"]["media_type"], mime.as_str());
        assert_eq!(content[1]["source"]["data"], data);
    }
}

#[test]
fn images_only_no_text_vision() {
    let mut m = Message::user("");
    m.user_image_blocks.push(
        crate::domain::conversation::value_objects::message::UserImageBlock::unchecked_for_tests(
            quecto_image::ImageMime::Jpeg,
            "data",
        ),
    );
    let content = build_user_content(&m);
    assert!(content.is_some());
    let arr = content.unwrap();
    assert!(arr.is_array());
    assert_eq!(arr.as_array().unwrap().len(), 1); // image only
}

// --- #2421: the gate is the application's; the provider sends what it gets ---

fn request_for<'a>(
    model: &'a str,
    messages: &'a [Message],
) -> crate::application::providers::ports::ChatRequest<'a> {
    crate::application::providers::ports::ChatRequest {
        trace: None,
        admission: None,
        messages,
        tools: &[],
        model,
        max_tokens: 256,
        temperature: 0.2,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

#[test]
fn every_model_is_sent_the_images_its_request_carries() {
    let mut m = Message::user("describe this");
    m.user_image_blocks.push(
        crate::domain::conversation::value_objects::message::UserImageBlock::unchecked_for_tests(
            quecto_image::ImageMime::Png,
            "cG5n",
        ),
    );
    let messages = [m];
    for model in [
        "unknown-future-model",
        "claude-instant-1",
        "claude-opus-4-8",
    ] {
        let (_, body) =
            crate::infrastructure::providers::anthropic::AnthropicProvider::build_request_body_public(
                &request_for(model, &messages),
            );
        let blocks = body["messages"][0]["content"]
            .as_array()
            .unwrap_or_else(|| panic!("{model}: a content array"))
            .clone();
        assert_eq!(blocks[1]["type"], "image", "{model}");
        assert_eq!(blocks[1]["source"]["data"], "cG5n", "{model}");
    }
}
