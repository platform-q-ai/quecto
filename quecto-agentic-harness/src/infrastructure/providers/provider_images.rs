//! Images as the OpenAI wires take them (#2421), shared by the Responses
//! (`codex_input`) and chat-completions (`openai_images`) serializers: an
//! inline `data:` URL per image, always at `"detail": "high"`.

use crate::domain::message::Message;

/// The detail every image is sent at (#2421 review D): `high` caps an
/// image at about 3,000 tokens on OpenAI's side, within the harness's
/// estimate of an image, where `original` (or none) would not.
pub(super) const DETAIL: &str = "high";

/// An image as both OpenAI wires take it inline.
pub(super) fn data_url(mime_type: &str, data: &str) -> String {
    format!("data:{mime_type};base64,{data}")
}

/// Every image `message` carries, as (MIME type, base64 data): a user's and
/// a tool result's alike, so none is left out whichever it is.
pub(super) fn images(message: &Message) -> Vec<(&str, &str)> {
    message
        .user_image_blocks
        .iter()
        .chain(&message.image_blocks)
        .map(|image| (image.mime_type(), image.data()))
        .collect()
}
