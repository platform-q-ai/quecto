use super::*;
use crate::interface::components::select_list::route_overlay_key;
use crate::interface::select_overlay::DOUBLE_ESC_WINDOW;

pub(super) fn rewind_preview(content: &str) -> String {
    let sanitized = strip_ansi_for_selection(content);
    crate::interface::utils::truncate_to_width(&sanitized, 48, Some("…"))
}

impl App {
    fn next_rewind_request_id(&mut self, kind: &str) -> String {
        self.rewind.request_seq = self.rewind.request_seq.wrapping_add(1);
        format!("rewind-{kind}-{}", self.rewind.request_seq)
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

    pub(super) fn open_rewind_selector(&mut self, data: &serde_json::Value) {
        let Some(messages) = data.get("messages").and_then(|v| v.as_array()) else {
            self.notify("No conversation history to rewind", NotifyLevel::Info);
            return;
        };

        let mut items = Vec::new();
        for (idx, message) in messages.iter().enumerate().rev() {
            if message.get("role").and_then(|v| v.as_str()) != Some("user") {
                continue;
            }
            let content = message
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let preview = rewind_preview(content);
            let turn_no = items.len() + 1;
            let label = if turn_no == 1 {
                format!("Previous turn: {preview}")
            } else {
                format!("{turn_no} turns ago: {preview}")
            };
            items.push(SelectItem {
                value: idx.to_string(),
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
        let Some(value) = route_overlay_key(&mut self.rewind.selector, key) else {
            return;
        };
        let Ok(message_index) = value.parse::<usize>() else {
            self.notify("Invalid rewind target", NotifyLevel::Error);
            return;
        };
        let id = self.next_rewind_request_id("to");
        self.rewind.pending_apply_id = Some(id.clone());
        self.send_command(Command::RewindTo {
            id: Some(id),
            message_index,
        });
    }
}
