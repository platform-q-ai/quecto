use crate::shell::app::App;
use crate::shell::app::{
    app_selection::{SelectionAnchor, display_col_to_char_idx},
    strip_ansi_for_selection,
};

impl App {
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
