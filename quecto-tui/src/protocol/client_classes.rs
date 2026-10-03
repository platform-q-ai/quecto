//! Command classification and harness support for the UDS client.
//!
//! Split from `client.rs` (750-line baseline): the writer-queue admission
//! classes (#1238 reserve, feed-liveness bypass) and the test-harness queue
//! constructor. A submodule of `client` so it can touch private fields.

use super::Command;
#[cfg(feature = "test-harness")]
use super::{COMMAND_WRITER_QUEUE_CAPACITY, CommandSender, mpsc};

/// Slots reserved so interactive user commands can still enqueue when
/// background fan-in has filled most of the ordered writer FIFO (#1238).
///
/// Background / housekeeping commands refuse to consume these last permits;
/// [`Command::is_interactive_user`] commands may use them. This does not
/// await capacity or reorder the FIFO — it only stops background traffic
/// from monopolizing the bound under sustained load.
pub const COMMAND_WRITER_USER_RESERVED: usize = 64;

/// The inner core of the user reserve that ONLY interactive commands may use.
///
/// Feed-liveness traffic ([`Command::is_feed_liveness`], i.e. `Sync`) is
/// admitted into the outer half of the reserve — refusing it under pressure
/// froze child feeds exactly while the parent was busy — but never past this
/// floor, so an unthrottled sync burst can not consume the slots protecting
/// prompt/steer/follow_up/abort (#1238, PR #1307 review).
pub const COMMAND_WRITER_INTERACTIVE_FLOOR: usize = 32;

impl Command {
    /// The wire `type` of the command (split from `client.rs` for its line cap).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Prompt { .. } => "prompt",
            Self::Steer { .. } => "steer",
            Self::FollowUp { .. } => "follow_up",
            Self::Abort { .. } => "abort",
            Self::GetState { .. } => "get_state",
            Self::GetMessages { .. } => "get_messages",
            Self::GetMessagesTail { .. } => "get_messages_tail",
            Self::GetMessage { .. } => "get_message",
            Self::GetSessionStats { .. } => "get_session_stats",
            Self::PersistSession { .. } => "persist_session",
            Self::GetToolCatalogue { .. } => "get_tool_catalogue",
            Self::SetToolPolicy { .. } => "set_tool_policy",
            Self::ListModels { .. } => "list_models",
            Self::RefreshModels { .. } => "refresh_models",
            Self::ListSessions { .. } => "list_sessions",
            Self::SearchSessionMetadata { .. } => "search_session_metadata",
            Self::NewSession { .. } => "new_session",
            Self::ResumeSession { .. } => "resume_session",
            Self::SetModel { .. } => "set_model",
            Self::SetEffort { .. } => "set_effort",
            Self::SetWorkflowAutomation { .. } => "set_workflow_automation",
            Self::ClearHistory { .. } => "clear_history",
            Self::RewindTo { .. } => "rewind_to",
            Self::GetSubagents { .. } => "get_subagents",
            Self::DeleteAllSubagents { .. } => "delete_all_subagents",
            Self::Sync { .. } => "sync",
        }
    }

    /// Interactive user actions that must not lose to background fan-in on
    /// the shared ordered writer queue (#1238).
    pub fn is_interactive_user(&self) -> bool {
        matches!(
            self,
            Self::Prompt { .. } | Self::Steer { .. } | Self::FollowUp { .. } | Self::Abort { .. }
        )
    }

    /// Commands that keep a feed live and must not be refused by the
    /// background reserve: a dropped `Sync` freezes the child feed until the
    /// next `ledger_advanced` (which may not come until the parent goes idle).
    pub fn is_feed_liveness(&self) -> bool {
        matches!(self, Self::Sync { .. })
    }

    /// Whether the command, serialized, fits one protocol frame: its payload
    /// plus the line's newline (the legacy line's cap counts it; a frame's
    /// is one byte looser) within [`super::MAX_LINE_BYTES`]. The writer drops
    /// an over-cap command with nothing on the wire, so a sender that must
    /// never lose one silently (a user message with images, #2425) asks
    /// first. A user message is measured, not serialized a second time.
    pub fn fits_one_frame(&self) -> bool {
        let payload = match self.user_message_len() {
            Some(len) => len,
            None => super::serialize_command(self).map_or(usize::MAX, |line| line.len() - 1),
        };
        payload.saturating_add(1) <= super::MAX_LINE_BYTES
    }

    /// The serialized length of a user message (`prompt` / `steer` /
    /// `follow_up`), measured without serializing it: the JSON serde writes
    /// for these variants, field by field, in declaration order; `None`
    /// for any other command.
    pub fn user_message_len(&self) -> Option<usize> {
        let (kind, id, message, behavior, images) = match self {
            Self::Prompt {
                id,
                message,
                streaming_behavior,
                images,
            } => ("prompt", id, message, streaming_behavior.as_deref(), images),
            Self::Steer {
                id,
                message,
                images,
            } => ("steer", id, message, None, images),
            Self::FollowUp {
                id,
                message,
                images,
            } => ("follow_up", id, message, None, images),
            _ => return None,
        };
        // `{"type":"<kind>"` … `}`
        let mut len = r#"{"type":""#.len() + kind.len() + 1 + 1;
        len += id
            .as_deref()
            .map_or(0, |id| r#","id":"#.len() + json_string_len(id));
        len += r#","message":"#.len() + json_string_len(message);
        len += behavior.map_or(0, |b| r#","streamingBehavior":"#.len() + json_string_len(b));
        len += match images.len() {
            0 => 0,
            count => {
                let each: usize = images
                    .iter()
                    .map(|image| {
                        r#"{"mimeType":"","data":""}"#.len()
                            + image.mime_type().len()
                            + image.data().len()
                    })
                    .sum();
                r#","images":[]"#.len() + each + (count - 1)
            }
        };
        Some(len)
    }
}

/// The length of `text` as a JSON string, quotes included, escaped as
/// `serde_json` escapes it: `"` `\` and the control characters with a
/// short form take two bytes, the other control characters six
/// (`\u00XX`), everything else its UTF-8 bytes.
fn json_string_len(text: &str) -> usize {
    let escaped: usize = text
        .chars()
        .map(|ch| match ch {
            '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
            '\u{0}'..='\u{1f}' => 6,
            _ => ch.len_utf8(),
        })
        .sum();
    escaped + 2
}

#[cfg(test)]
#[path = "client_frame_tests.rs"]
mod frame_tests;

#[cfg(feature = "test-harness")]
impl CommandSender {
    /// A sender over a production-sized writer queue, plus its receiver, for
    /// BDD/harness tests that pin the backpressure-reserve semantics.
    pub fn production_queue_for_tests() -> (Self, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel::<String>(COMMAND_WRITER_QUEUE_CAPACITY);
        (Self { tx }, rx)
    }
}
