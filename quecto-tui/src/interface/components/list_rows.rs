use crate::interface::theme;
use crate::interface::utils::{truncate_to_width, visible_width};

#[derive(Debug, Clone)]
pub struct ListRow {
    pub label: String,
    pub description: Option<String>,
    pub selected: bool,
    pub accent: bool,
    pub dim: bool,
    pub marker: Option<String>,
}

impl ListRow {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: None,
            selected: false,
            accent: false,
            dim: false,
            marker: None,
        }
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        let description = description.into();
        if !description.is_empty() {
            self.description = Some(description);
        }
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn accent(mut self, accent: bool) -> Self {
        self.accent = accent;
        self
    }

    pub fn dim(mut self, dim: bool) -> Self {
        self.dim = dim;
        self
    }

    pub fn marker(mut self, marker: impl Into<String>) -> Self {
        let marker = marker.into();
        if !marker.is_empty() {
            self.marker = Some(marker);
        }
        self
    }
}

pub fn row_label_width<'a>(rows: impl IntoIterator<Item = &'a ListRow>, cap: usize) -> usize {
    rows.into_iter()
        .map(|row| visible_width(&row.label))
        .max()
        .unwrap_or(10)
        .min(cap)
}

pub fn render_list_rows(rows: &[ListRow], width: usize, label_width: usize) -> Vec<String> {
    rows.iter()
        .map(|row| render_list_row(row, width, label_width))
        .collect()
}

fn render_list_row(row: &ListRow, width: usize, label_width: usize) -> String {
    let prefix = if row.selected { "→ " } else { "  " };
    let label = if row.dim {
        theme::dim(&row.label)
    } else if row.selected || row.accent {
        theme::accent(&row.label)
    } else {
        row.label.clone()
    };
    let marker = row.marker.as_deref().unwrap_or("");
    let label_vis = visible_width(&row.label) + visible_width(marker);

    let line = if let Some(description) = &row.description {
        let gap = label_width.saturating_sub(label_vis) + 2;
        let desc_start = visible_width(prefix) + label_vis + gap;
        let desc_width = width.saturating_sub(desc_start);
        if desc_width > 10 {
            let truncated_desc = truncate_to_width(description, desc_width, Some("…"));
            let spacing = " ".repeat(gap);
            format!(
                "{prefix}{label}{marker}{spacing}{}",
                theme::dim(&truncated_desc)
            )
        } else {
            format!("{prefix}{label}{marker}")
        }
    } else {
        format!("{prefix}{label}{marker}")
    };

    truncate_to_width(&line, width, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::ansi::strip_ansi;

    #[test]
    fn exact_width_keeps_description_without_overflow_indicator() {
        let rows = [ListRow::new("alpha").description("desc")];
        let rendered = render_list_rows(&rows, 30, row_label_width(&rows, 32));
        let plain = strip_ansi(&rendered[0]);
        assert_eq!(plain, "  alpha  desc");
        assert!(!plain.contains('…'));
    }

    #[test]
    fn one_column_too_narrow_truncates_description() {
        let rows = [ListRow::new("alpha").description("description is long")];
        let rendered = render_list_rows(&rows, 22, row_label_width(&rows, 32));
        let plain = strip_ansi(&rendered[0]);
        assert!(
            plain.ends_with('…'),
            "expected overflow indicator: {plain:?}"
        );
    }

    #[test]
    fn selection_accents_label_and_uses_indicator() {
        let rows = [ListRow::new("alpha").selected(true)];
        let rendered = render_list_rows(&rows, 20, row_label_width(&rows, 32));
        assert!(rendered[0].contains("→ "));
        assert!(rendered[0].contains("\u{1b}["));
    }

    #[test]
    fn unselected_row_has_plain_indicator() {
        let rows = [ListRow::new("alpha")];
        let rendered = render_list_rows(&rows, 20, row_label_width(&rows, 32));
        assert!(rendered[0].starts_with("  "));
        assert!(!rendered[0].contains("→ "));
    }
}
