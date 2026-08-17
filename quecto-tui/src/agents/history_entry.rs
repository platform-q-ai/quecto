use crate::components::chat::ChatEntry;
use crate::shell::app::App;

impl App {
    /// Map a backfilled/resumed history message to a chat entry: a ladder-demoted
    /// message carrying a stable id becomes a recallable [`ChatEntry::Stub`];
    /// anything else renders as a plain user/assistant line (#1061). Shared by the
    /// sub-agent/master backfill and the resume path so both recall identically.
    pub(crate) fn history_entry(
        text: String,
        id: Option<String>,
        stub: bool,
        is_user: bool,
        content_len: Option<usize>,
    ) -> ChatEntry {
        match (stub, id) {
            (true, Some(id)) => ChatEntry::Stub {
                id,
                is_user,
                text,
                content_len,
            },
            _ if is_user => ChatEntry::User { text },
            _ => ChatEntry::Assistant {
                text,
                thinking: Vec::new(),
                streaming: false,
            },
        }
    }

    pub(crate) fn history_entry_with_thinking(
        text: String,
        thinking: Vec<String>,
        id: Option<String>,
        stub: bool,
        is_user: bool,
        content_len: Option<usize>,
    ) -> ChatEntry {
        match (stub, id) {
            (true, Some(id)) => ChatEntry::Stub {
                id,
                is_user,
                text,
                content_len,
            },
            _ if is_user => ChatEntry::User { text },
            _ => ChatEntry::Assistant {
                text,
                thinking,
                streaming: false,
            },
        }
    }
}
