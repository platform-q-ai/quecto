//! Send/selection helpers split for line budget (#1465).

use super::app_selection::{SelectionAnchor, display_col_to_char_idx};
use super::*;

impl App {
    pub(super) fn send_command(&mut self, cmd: Command) -> bool {
        // Refuse a known-dead connection: the writer channel outlives the closed
        // event stream, so `try_send` would return `Ok` and the command vanish
        // into a dead socket. Bail silently and return false — callers that show
        // user feedback (e.g. `reset_session`) act on that (#1470 review).
        if !self.ac().agent_connected {
            self.refuse_disconnected_command(&cmd);
            return false;
        }
        // Enqueue synchronously in call order onto the client's FIFO writer; a
        // prior per-command `tokio::spawn` let bursts reach the agent reordered
        // or look incomplete to an observer draining mid-batch (#1060 review).
        if let Err(e) = self.ac().transport.try_send(&cmd) {
            // Roll back synchronously: the diagnostic side channel below is
            // best-effort, and if its receiver is gone we must not leave
            // pending history/resume/stub state stranded.
            self.rollback_failed_history_command(MASTER_CONNECTION_ID, &cmd, false);
            // Report without blocking the loop (a dropped notice is acceptable).
            let _ = self.command_send_failure_tx.try_send(CommandSendFailure {
                command: cmd,
                error: e.to_string(),
                connection: MASTER_CONNECTION_ID.to_string(),
            });
            return false;
        }
        true
    }

    // ── Mouse text selection (#528) ───────────────────────────────────

    /// Extract visible text from the rendered buffer between two selection anchors.
    pub(super) fn extract_selection(
        &self,
        start: &SelectionAnchor,
        end: &SelectionAnchor,
    ) -> String {
        // Normalize: ensure start ≤ end (top-to-bottom, left-to-right).
        let (start, end) = if (start.row, start.col) <= (end.row, end.col) {
            (start, end)
        } else {
            (end, start)
        };

        let lines = &self.last_rendered_lines;
        let (panel_width, divider_width, _) = self.frame_split();
        let body_start_col = panel_width.saturating_add(divider_width);
        let mut result = String::new();

        for row in start.row..=end.row {
            let row_idx = row as usize;
            if row_idx >= lines.len() {
                break;
            }
            let visible = strip_ansi_for_selection(&lines[row_idx]);
            let visible_width = crate::components::utils::visible_width(&visible);
            let chars: Vec<char> = visible.chars().collect();

            let col_start = if row == start.row {
                start.col as usize
            } else {
                0
            };
            let col_end = if row == end.row {
                end.col as usize
            } else {
                visible_width
            };

            let col_start = col_start.max(body_start_col).min(visible_width);
            let col_end = col_end.max(body_start_col).min(visible_width);

            let start_idx = display_col_to_char_idx(&chars, col_start);
            let end_idx = display_col_to_char_idx(&chars, col_end);
            let segment: String = chars[start_idx..end_idx].iter().collect();

            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&segment);
        }

        result
    }
}
