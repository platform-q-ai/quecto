//! Effective guard guidance is a provider view, never durable conversation state.
use crate::domain::agents::value_objects::turn_outcome::DECLARE_GUIDANCE;
use crate::domain::conversation::value_objects::message::{Message, Role};

/// Owns an exclusive loan until the provider request ends, including cancellation.
/// The image projection is nested inside this loan and restored first.
pub(super) struct GuidanceProjection<'a> {
    messages: &'a mut [Message],
    original: Option<(usize, String)>,
}

impl<'a> GuidanceProjection<'a> {
    pub(super) fn new(
        messages: &'a mut [Message],
        owned: Option<uuid::Uuid>,
        enabled: Option<bool>,
    ) -> Self {
        let mut projection = Self {
            messages,
            original: None,
        };
        if let (Some(id), Some(enabled)) = (owned, enabled)
            && let Some(index) = projection
                .messages
                .iter()
                .position(|message| message.id() == id && message.role == Role::System)
            && let Some(content) = effective_content(&projection.messages[index].content, enabled)
        {
            let message = &mut projection.messages[index];
            let original = std::mem::replace(&mut message.content, content);
            message.invalidate_token_cache();
            projection.original = Some((index, original));
        }
        projection
    }

    pub(super) fn messages(&mut self) -> &mut [Message] {
        self.messages
    }
}

impl Drop for GuidanceProjection<'_> {
    fn drop(&mut self) {
        if let Some((index, content)) = self.original.take() {
            self.messages[index].content = content;
            self.messages[index].invalidate_token_cache();
        }
    }
}

fn effective_content(canonical: &str, enabled: bool) -> Option<String> {
    if canonical.contains(DECLARE_GUIDANCE) == enabled {
        return None; // Stable policy: no string allocation or canonical mutation.
    }
    const OPEN: &str = "\n\n<extensions>\nContent length: ";
    const CLOSE: &str = "\n\n</extensions>";
    if let Some((prefix, framed)) = canonical.rsplit_once(OPEN)
        && let Some((_, body)) = framed.split_once(" bytes\n\n")
        && let Some(body) = body.strip_suffix(CLOSE)
    {
        let body = if enabled {
            format!("{body}{DECLARE_GUIDANCE}")
        } else {
            body.replace(DECLARE_GUIDANCE, "")
        };
        return Some(format!(
            "{prefix}{OPEN}{} bytes\n\n{body}{CLOSE}",
            body.len()
        ));
    }
    if enabled {
        Some(format!(
            "{canonical}{OPEN}{} bytes\n\n{DECLARE_GUIDANCE}{CLOSE}",
            DECLARE_GUIDANCE.len()
        ))
    } else {
        Some(canonical.replace(DECLARE_GUIDANCE, ""))
    }
}
