use super::*;
use crate::components::select_list::route_overlay_key;
use crate::components::select_overlay::DOUBLE_ESC_WINDOW;
use crate::protocol::session_payloads::{self, ResumedChatMessage};

pub(super) fn rewind_preview(content: &str) -> String {
    let sanitized = strip_ansi_for_selection(content);
    crate::components::utils::truncate_to_width(&sanitized, 48, Some("…"))
}

impl App {
    pub(super) fn next_rewind_request_id(&mut self, kind: &str) -> String {
        self.rewind.request_seq = self.rewind.request_seq.wrapping_add(1);
        format!(
            "{}rewind-{kind}-{}-{}",
            self.conn.id_namespace(),
            super::app_events::uuid_like(),
            self.rewind.request_seq
        )
    }

    pub(super) fn handle_idle_escape_for_rewind(&mut self) {
        let now = tokio::time::Instant::now();
        if self
            .rewind
            .last_idle_escape
            .is_some_and(|prev| now.duration_since(prev) <= DOUBLE_ESC_WINDOW)
        {
            self.rewind.last_idle_escape = None;
            let id = self.next_rewind_request_id("open");
            self.rewind.pending_open_id = Some(id.clone());
            self.send_command(Command::GetMessages {
                agent_id: None,
                id: Some(id),
                before: None,
            });
        } else {
            self.rewind.last_idle_escape = Some(now);
            self.notify(
                "Press Esc again to choose where to go back",
                NotifyLevel::Info,
            );
        }
    }

    /// Build the rewind selector from a `get_messages` response. With paged
    /// history (#1061) that response is the newest bounded page, so only user
    /// turns within it are offered as rewind targets — a deliberate trade
    /// against misapplying page-local positions to the full conversation.
    /// Paging inside the selector is a possible follow-up.
    pub(super) fn open_rewind_selector(&mut self, data: &serde_json::Value) {
        let Ok(messages) = session_payloads::parse_resumed_messages(data) else {
            self.notify("No conversation history to rewind", NotifyLevel::Info);
            return;
        };

        let mut items = Vec::new();
        for message in messages.iter().rev() {
            let ResumedChatMessage::User {
                text, id: Some(id), ..
            } = message
            else {
                continue;
            };
            // Target rewind by the message's STABLE id, not its page-local array
            // position: paged history (#1061) delivers only a bounded window, so an
            // array index here is not a valid index into the full server
            // conversation and could truncate the wrong turn (destructive). Messages
            // without an id (older harness) are not selectable rewind targets.
            let preview = rewind_preview(text);
            let turn_no = items.len() + 1;
            let label = if turn_no == 1 {
                format!("Previous turn: {preview}")
            } else {
                format!("{turn_no} turns ago: {preview}")
            };
            items.push(SelectItem {
                value: id.clone(),
                label,
                description: None,
            });
        }

        if items.is_empty() {
            self.notify("No previous user turns to rewind", NotifyLevel::Info);
            return;
        }
        self.rewind.selector = Some(SelectList::new(items, 10));
    }

    pub(super) fn handle_rewind_selector_key(&mut self, key: &Key) {
        let Some(message_id) = route_overlay_key(&mut self.rewind.selector, key) else {
            return;
        };
        let id = self.next_rewind_request_id("load");
        self.rewind.pending_load_id = Some(id.clone());
        self.rewind.pending_apply_message_id = Some(message_id.clone());
        self.rewind.pending_load_content.clear();
        self.rewind.pending_load_offset = 0;
        self.rewind.pending_load_content_len = None;
        self.send_command(Command::GetMessage {
            id: Some(id),
            message_id,
            agent_id: None,
            tool_call_id: None,
            offset: Some(0),
            limit: Some(super::app_paged_history::GET_MESSAGE_PAGE_BYTES),
        });
    }
}
