//! Recover full message or tool-call content (#1858): the selector a
//! client sends, the byte range it resolves to, and the recovered content.
//!
//! Ranges are UTF-8 safe: a requested offset or end inside a multi-byte
//! character moves back to the character boundary at or before it, and a
//! non-empty remainder always yields at least one whole character so a
//! client walking a message by `nextOffset` makes progress. The transport
//! narrows a range further only to fit its frame, through
//! [`Utf8Range::halve`].
use crate::domain::ids::{MessageId, ToolCallId};
use crate::domain::message::{Message, ToolCall};

/// One recovery request: a message, and which part of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryRequest {
    pub message_id: MessageId,
    pub selector: ContentSelector,
}

/// The allowlisted selections: the message (optionally a content range and
/// a visible-thinking offset) or one tool call's arguments (optionally a
/// range).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentSelector {
    Message {
        offset: Option<usize>,
        thinking_offset: Option<usize>,
        limit: Option<usize>,
    },
    ToolCallArguments {
        tool_call_id: ToolCallId,
        offset: Option<usize>,
        limit: Option<usize>,
    },
}

/// A byte range of a text on character boundaries: `start..end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Utf8Range {
    pub start: usize,
    pub end: usize,
}

fn nearest_char_boundary_at_or_before(s: &str, mut idx: usize) -> usize {
    idx = idx.min(s.len());
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn one_char_end(s: &str, start: usize) -> usize {
    s[start..]
        .char_indices()
        .nth(1)
        .map(|(idx, _)| start + idx)
        .unwrap_or(s.len())
}

impl Utf8Range {
    /// The range a client asked for over `text`: from `offset` (default 0,
    /// clamped to the text and to a character boundary) for `limit` bytes
    /// (default: the remainder), never past the end, and at least one whole
    /// character when anything remains. An offset at or past the end is the
    /// empty range at the end.
    pub fn requested(text: &str, offset: Option<usize>, limit: Option<usize>) -> Self {
        let len = text.len();
        let start = nearest_char_boundary_at_or_before(text, offset.unwrap_or(0));
        let remaining = len.saturating_sub(start);
        let requested = limit.unwrap_or(remaining).min(remaining);
        let requested_end =
            nearest_char_boundary_at_or_before(text, start.saturating_add(requested).min(len));
        let mut range = Self {
            start,
            end: requested_end,
        };
        if range.end == range.start && start < len {
            range.end = one_char_end(text, start);
        }
        range
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    /// The selected text.
    pub fn slice<'a>(&self, text: &'a str) -> &'a str {
        &text[self.start..self.end]
    }

    /// Narrow the range to fit a smaller budget: the end moves to the
    /// character boundary at or before the midpoint. Returns `false` when
    /// that would empty the range, in which case the range is instead the
    /// single character at `start` and cannot shrink further.
    pub fn halve(&mut self, text: &str) -> bool {
        let midpoint = self.start + (self.end - self.start) / 2;
        let end = nearest_char_boundary_at_or_before(text, midpoint);
        if end == self.start {
            self.end = one_char_end(text, self.start);
            return false;
        }
        self.end = end;
        true
    }
}

/// The recovered content: the message with its content range and thinking
/// offset, or one tool call's arguments with their range.
#[derive(Debug, Clone)]
pub enum RecoveredContent {
    Message {
        message: Box<Message>,
        /// The content range asked for (the whole content when no range was
        /// requested).
        range: Utf8Range,
        /// Where the visible-thinking page starts: the explicit thinking
        /// offset, else the content offset, else 0.
        thinking_offset: usize,
        /// Whether the client asked for a range at all; an unranged read may
        /// be answered whole when the transport can carry it.
        ranged: bool,
    },
    ToolCallArguments {
        message_id: MessageId,
        tool_call: ToolCall,
        range: Utf8Range,
    },
}

/// Why nothing was recovered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryError {
    /// Neither the ledger nor the live conversation holds the message.
    MessageNotFound(MessageId),
    /// The message holds no tool call with that id.
    ToolCallNotFound {
        message_id: MessageId,
        tool_call_id: ToolCallId,
    },
}

impl std::fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MessageNotFound(id) => write!(f, "message not found: {id}"),
            Self::ToolCallNotFound {
                message_id,
                tool_call_id,
            } => write!(
                f,
                "tool call {tool_call_id} not found in message {message_id}"
            ),
        }
    }
}

#[cfg(test)]
#[path = "message_recovery_tests.rs"]
mod tests;
