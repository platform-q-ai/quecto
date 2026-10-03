//! Which of a conversation's images a model is sent (#2421).
//!
//! One decision, made where the request is built, that every provider
//! shares: an image goes to a model only when the model's catalogue entry
//! declares `image` among its input modalities, and an animated GIF only
//! over a wire that takes one (the OpenAI wires document still GIFs only).
//! Each image a model does not take is sent as a short text marker after the
//! text of the message it belonged to: an image is never silently dropped.
//! The providers then serialize what they are given.
//!
//! What a message becomes depends on that message and the model alone,
//! never on the messages around it, so an earlier message is sent the same
//! way on every later request and the prompt cache keeps it (#2397).
//!
//! Nothing is copied (#2421 review L4): [`SentConversation`] takes the
//! withheld images out of their messages for as long as it lives and puts
//! every one back when it is dropped, however the request ends.

use base64::Engine;

use crate::domain::catalogue::TransportKind;
use crate::domain::message::{Message, UserImageBlock};
use crate::domain::tool::ImageBlock;

/// The input modality a model declares to take images.
const IMAGE_MODALITY: &str = "image";

/// What a model takes of a conversation's images.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImageInput {
    /// No image: each is sent as a marker. A model with no catalogue entry.
    #[default]
    NoImages,
    /// Every image but an animated GIF, which the OpenAI wires refuse.
    StillImages,
    /// Every image (the Anthropic Messages API).
    AllImages,
}

impl ImageInput {
    /// What a model whose entry declares `modalities`, reached over
    /// `transport`, takes: no image unless `image` is declared; then every
    /// image over the Anthropic wire, still ones over any other.
    pub fn declared(modalities: &[String], transport: &TransportKind) -> Self {
        let declares = modalities.iter().any(|declared| declared == IMAGE_MODALITY);
        match (declares, transport) {
            (false, _) => Self::NoImages,
            (true, TransportKind::AnthropicMessages) => Self::AllImages,
            (
                true,
                TransportKind::OpenAiCompletions
                | TransportKind::GoogleGenerativeAi
                | TransportKind::Unsupported { .. },
            ) => Self::StillImages,
        }
    }

    /// The marker an image of `mime_type` and `data` is sent as, for
    /// `model`; `None` when the image itself is sent.
    fn withheld(
        self,
        (mime_type, data): (&str, &str),
        model: &str,
        verdicts: &GifVerdicts,
    ) -> Option<String> {
        match self {
            Self::NoImages => Some(not_sent_marker(model)),
            Self::StillImages => verdicts
                .is_animated(mime_type, data)
                .then(|| animated_gif_marker(model)),
            Self::AllImages => None,
        }
    }
}

/// The text an image becomes for `model` when the model takes none.
pub fn not_sent_marker(model: &str) -> String {
    format!("[image not sent: {model} takes no image input]")
}

/// The text an animated GIF becomes for a model whose wire takes still
/// images only.
pub fn animated_gif_marker(model: &str) -> String {
    format!("[image not sent: animated GIF not supported by {model}]")
}

/// Whether `data` (base64) is a GIF of more than one image frame: a walk of
/// its blocks that stops at the second image descriptor. Anything that is
/// not a well-formed GIF up to there is not an animated one.
pub fn is_animated_gif(mime_type: &str, data: &str) -> bool {
    mime_type.eq_ignore_ascii_case("image/gif")
        && base64::engine::general_purpose::STANDARD
            .decode(data)
            .is_ok_and(|bytes| gif_frames(&bytes) > 1)
}

/// The image frames of `bytes` (a GIF), counted up to 2.
fn gif_frames(bytes: &[u8]) -> usize {
    const HEADER: usize = 13; // "GIF8?a" and the logical screen descriptor
    let mut frames = 0;
    let Some(flags) = bytes.get(10).filter(|_| bytes.starts_with(b"GIF8")) else {
        return frames;
    };
    let mut at = HEADER + colour_table(*flags);
    while frames < 2 {
        let next = match bytes.get(at) {
            Some(0x2C) => {
                frames += 1;
                // The descriptor's 9 bytes, its colour table, the LZW size.
                let local = bytes.get(at + 9).copied().map_or(0, colour_table);
                skip_sub_blocks(bytes, at + 10 + local + 1)
            }
            Some(0x21) => skip_sub_blocks(bytes, at + 2),
            Some(_) | None => None,
        };
        match next {
            Some(after) => at = after,
            None => break,
        }
    }
    frames
}

/// The bytes of the colour table a GIF `flags` byte declares.
fn colour_table(flags: u8) -> usize {
    match flags & 0x80 {
        0 => 0,
        _ => 3 << ((flags & 0x07) + 1),
    }
}

/// Where the data sub-blocks starting at `at` end (past their terminator).
fn skip_sub_blocks(bytes: &[u8], mut at: usize) -> Option<usize> {
    loop {
        let size = usize::from(*bytes.get(at)?);
        at += 1;
        match size {
            0 => return Some(at),
            _ => at += size,
        }
    }
}

/// What the animated-GIF walk found for each GIF already seen (#2421
/// round 2 nit 1), so a request walks a GIF of the history once, not on
/// every request.
#[derive(Debug, Default)]
pub struct GifVerdicts {
    walks: std::sync::atomic::AtomicUsize,
}

impl GifVerdicts {
    /// Whether the `mime_type` image `data` encodes is an animated GIF.
    pub fn is_animated(&self, mime_type: &str, data: &str) -> bool {
        self.walks
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        is_animated_gif(mime_type, data)
    }

    /// How many GIFs have been walked.
    pub fn walks(&self) -> usize {
        self.walks.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// What one message lent the request: where its text ended, and each
/// withheld image with the place it came from.
struct Withheld {
    index: usize,
    text_len: usize,
    tool_images: Vec<(usize, ImageBlock)>,
    user_images: Vec<(usize, UserImageBlock)>,
}

/// The conversation as a model is sent it, for as long as this lives: each
/// image the model does not take is out of its message, with a marker after
/// the message's text in its place. Dropping it puts every message back.
pub struct SentConversation<'m> {
    messages: &'m mut [Message],
    withheld: Vec<Withheld>,
}

impl<'m> SentConversation<'m> {
    pub fn new(
        messages: &'m mut [Message],
        model: &str,
        input: ImageInput,
        verdicts: &GifVerdicts,
    ) -> Self {
        let withheld = match input {
            ImageInput::AllImages => Vec::new(),
            ImageInput::NoImages | ImageInput::StillImages => messages
                .iter_mut()
                .enumerate()
                .filter_map(|(index, message)| withhold(index, message, (model, input), verdicts))
                .collect(),
        };
        Self { messages, withheld }
    }

    /// The messages as they are sent.
    pub fn messages(&self) -> &[Message] {
        self.messages
    }
}

impl Drop for SentConversation<'_> {
    fn drop(&mut self) {
        for lent in self.withheld.drain(..) {
            let message = &mut self.messages[lent.index];
            message.content.truncate(lent.text_len);
            let sent = std::mem::take(&mut message.image_blocks);
            message.image_blocks = merge(sent, lent.tool_images);
            let sent = std::mem::take(&mut message.user_image_blocks);
            message.user_image_blocks = merge(sent, lent.user_images);
            message.invalidate_token_cache();
        }
    }
}

/// Take out of `message` the images `input` does not send, marking each;
/// `None` when it keeps them all.
fn withhold(
    index: usize,
    message: &mut Message,
    (model, input): (&str, ImageInput),
    verdicts: &GifVerdicts,
) -> Option<Withheld> {
    let mut markers = Vec::new();
    let (kept, tool_images) = split(
        std::mem::take(&mut message.image_blocks),
        |image| input.withheld((image.mime_type, &image.data), model, verdicts),
        &mut markers,
    );
    message.image_blocks = kept;
    let (kept, user_images) = split(
        std::mem::take(&mut message.user_image_blocks),
        |image| input.withheld((&image.mime_type, &image.data), model, verdicts),
        &mut markers,
    );
    message.user_image_blocks = kept;
    let text_len = message.content.len();
    match markers.is_empty() {
        true => None,
        false => {
            for marker in &markers {
                match message.content.is_empty() {
                    true => {}
                    false => message.content.push('\n'),
                }
                message.content.push_str(marker);
            }
            message.invalidate_token_cache();
            debug_assert_eq!(
                tool_images.len() + user_images.len(),
                markers.len(),
                "one marker per withheld image"
            );
            Some(Withheld {
                index,
                text_len,
                tool_images,
                user_images,
            })
        }
    }
}

/// `images` split into those sent and those withheld (with their places),
/// each withheld one's marker added to `markers`.
fn split<T>(
    images: Vec<T>,
    marker: impl Fn(&T) -> Option<String>,
    markers: &mut Vec<String>,
) -> (Vec<T>, Vec<(usize, T)>) {
    let mut kept = Vec::with_capacity(images.len());
    let mut withheld = Vec::new();
    for (place, image) in images.into_iter().enumerate() {
        match marker(&image) {
            Some(text) => {
                markers.push(text);
                withheld.push((place, image));
            }
            None => kept.push(image),
        }
    }
    (kept, withheld)
}

/// `sent` and `withheld` back in their original order.
fn merge<T>(sent: Vec<T>, withheld: Vec<(usize, T)>) -> Vec<T> {
    let total = sent.len() + withheld.len();
    let mut sent = sent.into_iter();
    let mut withheld = withheld.into_iter().peekable();
    let mut images = Vec::with_capacity(total);
    for place in 0..total {
        let image = match withheld.next_if(|(at, _)| *at == place) {
            Some((_, image)) => Some(image),
            None => sent.next(),
        };
        images.extend(image);
    }
    debug_assert_eq!(images.len(), total, "every image is put back");
    images
}

#[cfg(test)]
#[path = "image_input_tests.rs"]
mod tests;
