//! #2421: an image goes to a model only when the model declares image input;
//! otherwise each image is a marker in the message it belonged to. The
//! conversation is never copied, and gets every image back afterwards.
use super::*;
use crate::domain::message::UserImageBlock;
use crate::domain::tool::ImageBlock;
use base64::Engine;

const MODEL: &str = "fireworks/glm-5p2";

fn modalities(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn user_with_images(text: &str, count: usize) -> Message {
    let mut message = Message::user(text);
    message.user_image_blocks = (0..count)
        .map(|index| {
            UserImageBlock::unchecked_for_tests(
                quecto_image::ImageMime::Png,
                format!("cG5n{index}"),
            )
        })
        .collect();
    message
}

fn tool_with_image(id: &str, text: &str) -> Message {
    let mut message = Message::tool(id, text);
    message.tool_name = Some("read".into());
    message.image_blocks = vec![ImageBlock::new("image/jpeg", "anBn")];
    message
}

/// A GIF of `frames` 1x1 frames, as base64.
fn gif(frames: usize) -> String {
    quecto_image::encode(&quecto_image::samples::animated_gif(frames))
}

fn tool_with_gif(id: &str, frames: usize) -> Message {
    let mut message = Message::tool(id, "Read image file a.gif");
    message.image_blocks = vec![
        ImageBlock::new("image/png", "cG5n"),
        ImageBlock::new("image/gif", gif(frames)),
        ImageBlock::new("image/jpeg", "anBn"),
    ];
    message
}

#[test]
fn a_model_declaring_image_input_takes_every_image_over_anthropic() {
    let declared = modalities(&["text", "image"]);
    assert_eq!(
        ImageInput::declared(&declared, &TransportKind::AnthropicMessages),
        ImageInput::AllImages
    );
}

#[test]
fn a_model_declaring_image_input_takes_still_images_over_openai() {
    let declared = modalities(&["text", "image"]);
    assert_eq!(
        ImageInput::declared(&declared, &TransportKind::OpenAiCompletions),
        ImageInput::StillImages
    );
}

#[test]
fn a_model_that_declares_no_image_input_takes_none() {
    for transport in [
        TransportKind::AnthropicMessages,
        TransportKind::OpenAiCompletions,
    ] {
        for declared in [
            modalities(&["text"]),
            modalities(&["text", "audio"]),
            vec![],
        ] {
            assert_eq!(
                ImageInput::declared(&declared, &transport),
                ImageInput::NoImages,
                "{declared:?} over {transport:?}"
            );
        }
    }
    assert_eq!(ImageInput::default(), ImageInput::NoImages, "no entry");
}

#[test]
fn the_markers_name_the_model() {
    assert_eq!(
        not_sent_marker(MODEL),
        "[image not sent: fireworks/glm-5p2 takes no image input]"
    );
    assert_eq!(
        animated_gif_marker(MODEL),
        "[image not sent: animated GIF not supported by fireworks/glm-5p2]"
    );
}

/// Whether a GIF is animated is `quecto_image`'s (#2422 review); the
/// verdict asks it only of an image typed exactly `image/gif`, as the wire
/// spells it.
#[test]
fn only_an_image_typed_exactly_image_gif_is_walked() {
    let verdicts = GifVerdicts::default();
    assert!(verdicts.is_animated("image/gif", &gif(2)));
    assert!(!verdicts.is_animated("image/gif", &gif(1)));
    for mime in ["IMAGE/GIF", "image/Gif", "image/png"] {
        assert!(!verdicts.is_animated(mime, &gif(2)), "{mime}");
    }
}

#[test]
fn a_model_that_takes_every_image_is_sent_the_conversation_as_it_is() {
    let mut messages = vec![user_with_images("look", 1), tool_with_gif("c1", 2)];
    let stored = messages.as_ptr();
    let sent = SentConversation::new(
        &mut messages,
        MODEL,
        ImageInput::AllImages,
        &GifVerdicts::default(),
    );
    assert_eq!(sent.messages().as_ptr(), stored, "nothing is copied");
    assert_eq!(sent.messages()[0].user_image_blocks.len(), 1);
    assert_eq!(sent.messages()[1].image_blocks.len(), 3);
}

#[test]
fn each_user_image_becomes_a_marker_after_the_text() {
    let mut messages = vec![user_with_images("compare these", 2)];
    let marker = not_sent_marker(MODEL);
    {
        let sent = SentConversation::new(
            &mut messages,
            MODEL,
            ImageInput::NoImages,
            &GifVerdicts::default(),
        );
        assert_eq!(
            sent.messages()[0].content,
            format!("compare these\n{marker}\n{marker}")
        );
        assert!(sent.messages()[0].user_image_blocks.is_empty());
    }
    assert_eq!(messages[0].content, "compare these", "put back");
    assert_eq!(messages[0].user_image_blocks.len(), 2);
    assert_eq!(messages[0].user_image_blocks[1].data(), "cG5n1", "in order");
}

#[test]
fn an_image_with_no_text_becomes_the_marker_alone() {
    let mut messages = vec![user_with_images("", 1)];
    let sent = SentConversation::new(
        &mut messages,
        MODEL,
        ImageInput::NoImages,
        &GifVerdicts::default(),
    );
    assert_eq!(sent.messages()[0].content, not_sent_marker(MODEL));
}

#[test]
fn messages_without_images_are_neither_copied_nor_changed() {
    let mut messages = vec![
        Message::user("read it"),
        tool_with_image("c1", "Read image file a.jpg"),
        Message::tool("c2", "plain"),
    ];
    let stored = messages.as_ptr();
    let texts = [messages[0].content.as_ptr(), messages[2].content.as_ptr()];
    let sent = SentConversation::new(
        &mut messages,
        MODEL,
        ImageInput::NoImages,
        &GifVerdicts::default(),
    );
    assert_eq!(
        sent.messages().as_ptr(),
        stored,
        "the conversation is not copied"
    );
    assert_eq!(sent.messages()[0].content.as_ptr(), texts[0]);
    assert_eq!(sent.messages()[2].content.as_ptr(), texts[1]);
    assert_eq!(
        sent.messages()[1].content,
        format!("Read image file a.jpg\n{}", not_sent_marker(MODEL))
    );
    assert!(sent.messages()[1].image_blocks.is_empty());
    assert_eq!(sent.messages()[1].tool_call_id.as_deref(), Some("c1"));
}

#[test]
fn a_still_image_model_is_sent_every_image_but_an_animated_gif() {
    let mut messages = vec![tool_with_gif("c1", 2), tool_with_gif("c2", 1)];
    {
        let sent = SentConversation::new(
            &mut messages,
            MODEL,
            ImageInput::StillImages,
            &GifVerdicts::default(),
        );
        let animated = &sent.messages()[0];
        assert_eq!(
            animated.content,
            format!("Read image file a.gif\n{}", animated_gif_marker(MODEL))
        );
        let kept: Vec<&str> = animated.image_blocks.iter().map(|i| i.mime_type).collect();
        assert_eq!(kept, ["image/png", "image/jpeg"]);
        let still = &sent.messages()[1];
        assert_eq!(still.content, "Read image file a.gif");
        assert_eq!(still.image_blocks.len(), 3);
    }
    let restored: Vec<&str> = messages[0]
        .image_blocks
        .iter()
        .map(|i| i.mime_type)
        .collect();
    assert_eq!(
        restored,
        ["image/png", "image/gif", "image/jpeg"],
        "in order"
    );
    assert_eq!(messages[0].content, "Read image file a.gif");
}

#[test]
fn the_token_estimate_is_the_conversations_again_afterwards() {
    let mut messages = vec![user_with_images("look", 2)];
    let before = messages[0].estimated_tokens();
    {
        let sent = SentConversation::new(
            &mut messages,
            MODEL,
            ImageInput::NoImages,
            &GifVerdicts::default(),
        );
        assert_ne!(sent.messages()[0].estimated_tokens(), before);
    }
    assert_eq!(messages[0].estimated_tokens(), before);
}

#[test]
fn a_message_is_marked_the_same_whatever_follows_it() {
    let mut earlier = vec![user_with_images("look", 1)];
    let mut later = earlier.clone();
    later.push(Message::assistant("seen", vec![]));
    later.push(tool_with_image("c1", "more"));
    let first = SentConversation::new(
        &mut earlier,
        MODEL,
        ImageInput::NoImages,
        &GifVerdicts::default(),
    )
    .messages()[0]
        .content
        .clone();
    let second = SentConversation::new(
        &mut later,
        MODEL,
        ImageInput::NoImages,
        &GifVerdicts::default(),
    )
    .messages()[0]
        .content
        .clone();
    assert_eq!(first, second);
}

/// #2421 round 2 nit 1: a GIF is walked once, however many requests send
/// the conversation that holds it.
#[test]
fn a_gif_is_walked_once_across_requests() {
    let verdicts = GifVerdicts::default();
    let mut messages = vec![tool_with_gif("c1", 2), tool_with_gif("c2", 1)];
    for _ in 0..3 {
        let sent = SentConversation::new(&mut messages, MODEL, ImageInput::StillImages, &verdicts);
        assert_eq!(
            sent.messages()[0].image_blocks.len(),
            2,
            "animated: withheld"
        );
        assert_eq!(sent.messages()[1].image_blocks.len(), 3, "still: sent");
    }
    assert_eq!(verdicts.walks(), 2, "each GIF once");
}

/// #2421 round 3 L3: verdicts are kept by a digest of the image, so two
/// different GIFs of the same length, one animated and one still, each get
/// their own.
#[test]
fn gifs_of_the_same_length_get_their_own_verdicts() {
    let animated = gif(2);
    let still = {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(gif(1))
            .unwrap();
        let (body, trailer) = bytes.split_at(bytes.len() - 1);
        let mut padded = body.to_vec();
        // A comment as long as the second frame (graphic control, image
        // descriptor and data: 23 bytes).
        padded.extend([0x21, 0xFE, 19]);
        padded.extend([b'x'; 19]);
        padded.push(0);
        padded.extend(trailer);
        base64::engine::general_purpose::STANDARD.encode(padded)
    };
    assert_eq!(animated.len(), still.len());
    let verdicts = GifVerdicts::default();
    assert!(verdicts.is_animated("image/gif", &animated));
    assert!(!verdicts.is_animated("image/gif", &still));
    assert!(verdicts.is_animated("image/gif", &animated));
    assert_eq!(verdicts.walks(), 2);
}
