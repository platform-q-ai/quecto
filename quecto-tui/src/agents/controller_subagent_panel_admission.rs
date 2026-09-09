//! Admission-first name row layout, isolated from panel navigation.
use super::*;

impl App {
    pub(super) fn panel_admission_name_line(
        &self,
        row: &PanelRow,
        selbar: &str,
        active: bool,
        width: usize,
        now: tokio::time::Instant,
    ) -> String {
        use crate::components::utils::{truncate_to_width, visible_width};
        let label = row
            .admission
            .as_deref()
            .expect("admission row requires a label");
        let admission = format!("{} {}", theme::ADMISSION_INDICATOR, label);
        let admission_width = visible_width(&admission);
        if width >= admission_width + 4 {
            // Reserve cursor, right gutter, admission separator and one name cell.
            let mut remaining = width - admission_width - 4;
            let observer = self.panel_row_observer(row.id.as_deref()).unwrap_or("");
            let observer = if visible_width(observer) <= remaining {
                remaining -= visible_width(observer);
                observer
            } else {
                ""
            };
            let timer = if row.is_environment() {
                String::new()
            } else {
                self.panel_row_timer(row.id.as_deref(), now)
            };
            let timer = if visible_width(&timer) < remaining {
                remaining -= visible_width(&timer) + 1;
                format!(" {timer}")
            } else {
                String::new()
            };
            let prefix = truncate_to_width(&row.prefix, remaining, None);
            remaining -= visible_width(&prefix);
            let name =
                truncate_to_width(&sanitize_panel_label(&row.label), remaining + 1, Some("…"));
            let mut name = status_colored_name(&row.status, &name);
            if active {
                name = theme::bold(&name);
            }
            let content = format!(
                "{selbar}{}{name} {}{observer}",
                theme::dim(&prefix),
                theme::dim(&admission)
            );
            let used = visible_width(&content) + visible_width(&timer) + 1;
            debug_assert!(used <= width, "admission row exceeded its reserved columns");
            pad_cell(
                &format!(
                    "{content}{}{} ",
                    " ".repeat(width.saturating_sub(used)),
                    theme::dim(&timer)
                ),
                width,
            )
        } else {
            let fallback = if width >= admission_width {
                admission.as_str()
            } else if width >= visible_width(theme::ADMISSION_INDICATOR) {
                theme::ADMISSION_INDICATOR
            } else if width == 1 {
                "?"
            } else {
                ""
            };
            pad_cell(&theme::dim(fallback), width)
        }
    }
}
