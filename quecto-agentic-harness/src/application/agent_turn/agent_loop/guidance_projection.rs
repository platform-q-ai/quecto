//! Effective guard guidance is a provider view, never durable conversation state.
use crate::domain::agents::value_objects::turn_outcome::DECLARE_GUIDANCE;
use crate::domain::conversation::value_objects::message::{Message, Role};

/// Append provenance captured before guidance is written, never inferred from text.
/// The snapshot only validates an already-owned range after binding/reload.
#[derive(Clone)]
pub(crate) struct GuidanceAppend {
    canonical: std::sync::Arc<str>,
    range: std::ops::Range<usize>,
    message_id: Option<uuid::Uuid>,
    bound: bool,
}

impl GuidanceAppend {
    pub(crate) fn append_to(snippets: &mut String) -> Self {
        match snippets.as_str() {
            "" => {}
            _ => snippets.push_str("\n\n"),
        }
        let start = snippets.len();
        snippets.push_str(DECLARE_GUIDANCE);
        debug_assert_eq!(snippets.get(start..), Some(DECLARE_GUIDANCE));
        Self {
            canonical: snippets.as_str().into(),
            range: start..snippets.len(),
            message_id: None,
            bound: false,
        }
    }

    pub(crate) fn bind(&mut self, message_id: Option<uuid::Uuid>) {
        match (self.bound, self.message_id == message_id) {
            (false, _) | (true, true) => self.message_id = message_id,
            (true, false) => self.message_id = None,
        }
        self.bound = true;
    }

    fn valid_range(&self, message: &Message) -> Option<std::ops::Range<usize>> {
        if self.message_id == Some(message.id())
            && message.role == Role::System
            && self.range.start <= self.range.end
            && message.content.is_char_boundary(self.range.start)
            && message.content.is_char_boundary(self.range.end)
            && message.content.get(self.range.clone()) == Some(DECLARE_GUIDANCE)
            && message.content == self.canonical.as_ref()
        {
            return Some(self.range.clone());
        }
        None
    }
}

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
        appended: Option<&GuidanceAppend>,
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
            && let Some(content) = effective_content(&projection.messages[index], enabled, appended)
        {
            let message = &mut projection.messages[index];
            let original = std::mem::replace(&mut message.content, content);
            message.invalidate_token_cache();
            projection.original = Some((index, original));
        }
        projection
    }

    /// Price the same owned provider view before pruning, without lending or
    /// mutating the canonical head. Unowned heads/manifests add no guidance.
    pub(super) fn token_change(
        messages: &[Message],
        owned: Option<uuid::Uuid>,
        enabled: Option<bool>,
        appended: Option<&GuidanceAppend>,
    ) -> (usize, usize) {
        if let (Some(id), Some(enabled)) = (owned, enabled)
            && let Some(message) = messages
                .iter()
                .find(|message| message.id() == id && message.role == Role::System)
            && let Some(content) = effective_content(message, enabled, appended)
        {
            let canonical = message.estimated_tokens();
            let mut projected = message.clone();
            projected.content = content;
            projected.invalidate_token_cache();
            return (canonical, projected.estimated_tokens());
        }
        (0, 0)
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

fn effective_content(
    message: &Message,
    enabled: bool,
    appended: Option<&GuidanceAppend>,
) -> Option<String> {
    let canonical = &message.content;
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
    } else if let Some(range) = appended.and_then(|owned| owned.valid_range(message)) {
        let mut content = canonical.clone();
        content.replace_range(range, "");
        Some(content)
    } else {
        None
    }
}

#[cfg(test)]
#[path = "guidance_projection_tests.rs"]
mod tests;
