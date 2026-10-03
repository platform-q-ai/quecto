//! Which of a conversation's images a model is sent (#2421).
//!
//! One decision, made where the request is built, that every provider
//! shares: an image goes to a model only when the model's catalogue entry
//! declares `image` among its input modalities, and an animated GIF only
//! over a wire that takes one (the OpenAI wires document still GIFs only).
//! Each image a model does not take is sent as a short text marker after the
//! text of the message it belonged to: an image is never silently dropped.
//! So is an image a resumed session could not load (#2424), for any model.
//! The providers then serialize what they are given.
//!
//! What a message becomes depends on that message and the model alone,
//! never on the messages around it, so an earlier message is sent the same
//! way on every later request and the prompt cache keeps it (#2397).
//!
//! Nothing is copied (#2421 review L4): [`SentConversation`] takes the
//! withheld images out of their messages for as long as it lives and puts
//! every one back when it is dropped, however the request ends.

use quecto_image::ImageMime;

use crate::domain::catalogue::TransportKind;
use crate::domain::conversation::stored_images::unavailable_marker;
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

    /// The marker `image` is sent as, for `model`; `None` when the image
    /// itself is sent.
    fn withheld(self, image: &ImageBlock, model: &str, verdicts: &GifVerdicts) -> Option<String> {
        match self {
            Self::NoImages => Some(not_sent_marker(model)),
            Self::StillImages => verdicts
                .is_animated(image)
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

/// What the animated-GIF walk found for each GIF already seen (#2421
/// round 2 nit 1), keyed by the SHA-256 of its base64 (round 3 L3: a
/// crafted collision cannot pass an animated GIF off as a still one), the
/// digest the block caches (#2423: never hashed again here), so a request
/// walks a GIF of the history once, not on every request.
#[derive(Debug, Default)]
pub struct GifVerdicts {
    known: std::sync::Mutex<std::collections::HashMap<String, bool>>,
    #[cfg(any(test, feature = "test-support"))]
    walks: std::sync::atomic::AtomicUsize,
}

/// The most verdicts kept; past it they are forgotten and walked again.
pub const MAX_GIF_VERDICTS: usize = 1_024;

impl GifVerdicts {
    /// Whether `image` is an animated GIF; a GIF is walked the first time
    /// it is asked about. Only a GIF is walked; whether it is animated is
    /// `quecto_image`'s format fact, cached here by the block's digest.
    pub fn is_animated(&self, image: &ImageBlock) -> bool {
        match image.mime() {
            ImageMime::Png | ImageMime::Jpeg | ImageMime::Webp => false,
            ImageMime::Gif => {
                let key = image.sha256();
                let mut known = self.known.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(verdict) = known.get(key) {
                    return *verdict;
                }
                #[cfg(any(test, feature = "test-support"))]
                self.walks
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let verdict = quecto_image::is_animated_gif(image.data());
                if known.len() >= MAX_GIF_VERDICTS {
                    known.clear();
                }
                known.insert(key.to_owned(), verdict);
                verdict
            }
        }
    }

    /// How many GIFs have been walked (tests).
    #[cfg(any(test, feature = "test-support"))]
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
        // #2424: an image a resumed session could not load is a marker for
        // every model, so every model's request visits the messages with one.
        let visits = |message: &Message| match input {
            ImageInput::AllImages => !message.unloaded_images.is_empty(),
            ImageInput::NoImages | ImageInput::StillImages => true,
        };
        let withheld = messages
            .iter_mut()
            .enumerate()
            .filter(|(_, message)| visits(message))
            .filter_map(|(index, message)| withhold(index, message, (model, input), verdicts))
            .collect();
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
        |image| input.withheld(image, model, verdicts),
        &mut markers,
    );
    message.image_blocks = kept;
    let (kept, user_images) = split(
        std::mem::take(&mut message.user_image_blocks),
        |image| input.withheld(image, model, verdicts),
        &mut markers,
    );
    message.user_image_blocks = kept;
    let unloaded = message.unloaded_images.iter();
    markers.extend(unloaded.map(|image| unavailable_marker(&image.reference.sha256)));
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
                tool_images.len() + user_images.len() + message.unloaded_images.len(),
                markers.len(),
                "one marker per withheld or unloaded image"
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
