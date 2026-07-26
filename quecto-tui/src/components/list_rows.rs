//! Shared list-row renderer for the TUI's list/overlay surfaces (#997): the
//! select list, slash-command autocomplete, `@files` autocomplete and model
//! selector all draw the same shape — a windowed list, a `→ ` prefix on the
//! selected row, an accent label, an optional dim description column, and a
//! dim `(sel/total)` indicator on overflow. Windowing AND the indicator live
//! here ONCE; surfaces differ only via indent, [`DescriptionMode`], [`ListRow`].

use crate::components::list_navigator::ListNavigator;
use crate::interface::theme;
use crate::interface::utils::{truncate_to_width, visible_width};

/// One renderable row: the display label exactly as the surface shows it
/// (with any `/`/`@` sigil), plus per-row decorations.
#[derive(Debug, Clone)]
pub struct ListRow {
    /// Display label (unstyled); the helper applies accent/dim styling.
    pub label: String,
    /// Dim right-hand column (command description, provider name), if any.
    pub description: Option<String>,
    /// Suffix drawn after the label but OUTSIDE the alignment column (the
    /// model selector's ` ●` current-model marker); empty when absent.
    pub marker: &'static str,
    /// Render the label dim and never accent it (`@files` loading placeholder).
    pub dim_label: bool,
}

impl ListRow {
    /// A plain row with no description, marker or dimming.
    pub fn plain(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: None,
            marker: "",
            dim_label: false,
        }
    }
}

/// How the dim description column is laid out — the one real difference
/// between the four surfaces.
#[derive(Debug, Clone, Copy)]
pub enum DescriptionMode {
    /// Column aligned to the widest VISIBLE label, capped at 32 (#757:
    /// off-screen items are never drawn, so widening the column for them
    /// would waste a full-list scan every frame). The description is truncated
    /// to the remaining width and dropped below `min_desc_width` (select list).
    AlignedWindow { min_desc_width: usize },
    /// Column aligned to a caller-cached label width (the model selector's
    /// `cached_max_label_width`, recomputed only on filter change — never a
    /// per-frame full-filtered-list scan, #757). The description is always
    /// drawn; the whole LINE is truncated to `width`. `label_width: 0` gives a
    /// fixed two-space gap (the slash-command autocomplete's layout).
    AlignedCached { label_width: usize },
}

/// Render the full list surface for `items`: window to the navigator's visible
/// range, build each visible row with `to_row`, and emit the rows plus — when
/// the window overflows — the dim `  (selected+1/total)` indicator line
/// (always two cells from the left margin, regardless of `indent`).
///
/// Behavior contract (characterized against the pre-#997 renderers): selected
/// row prefix `→ ` after `indent`, others two spaces; selected label accented
/// unless `dim_label` (dim labels are never accented); `marker` sits between
/// label and description gap, OUTSIDE the alignment column (the description
/// shifts by the marker width on that row, exactly as today); every emitted
/// line fits `width`.
#[expect(clippy::too_many_arguments, reason = "single shared entry point")]
pub fn render_windowed<T>(
    items: &[T],
    nav: &ListNavigator,
    max_visible: usize,
    width: usize,
    indent: &str,
    mode: DescriptionMode,
    to_row: impl Fn(&T) -> ListRow,
) -> Vec<String> {
    let range = nav.visible_range(items.len(), max_visible);
    let selected = nav.selected();
    let rows: Vec<ListRow> = items[range.clone()].iter().map(to_row).collect();
    let mut lines = Vec::with_capacity(rows.len() + 1);

    // `AlignedWindow` column: widest VISIBLE label only, capped at 32 (#757)
    // — `rows` holds just the window, never the full list.
    let window_label_width = match mode {
        DescriptionMode::AlignedWindow { .. } => rows
            .iter()
            .map(|r| visible_width(&r.label))
            .max()
            .unwrap_or(10)
            .min(32),
        _ => 0,
    };

    for (offset, row) in rows.iter().enumerate() {
        let is_sel = range.start + offset == selected;
        let prefix = if is_sel { "→ " } else { "  " };
        let label = if row.dim_label {
            theme::dim(&row.label)
        } else if is_sel {
            theme::accent(&row.label)
        } else {
            row.label.clone()
        };
        let label_vis = visible_width(&row.label);
        let mut line = format!("{}{}{}{}", indent, prefix, label, row.marker);

        // Dim column; `AlignedWindow` drops it below `min_desc_width`.
        if let Some(desc) = &row.description {
            let column = match mode {
                DescriptionMode::AlignedWindow { min_desc_width } => {
                    let gap = window_label_width.saturating_sub(label_vis) + 2;
                    let desc_start = visible_width(indent) + 2 + label_vis + gap;
                    let desc_width = width.saturating_sub(desc_start + 1);
                    (desc_width > min_desc_width)
                        .then(|| (gap, truncate_to_width(desc, desc_width, Some(""))))
                }
                DescriptionMode::AlignedCached { label_width } => {
                    Some((label_width.saturating_sub(label_vis) + 2, desc.clone()))
                }
            };
            if let Some((gap, desc)) = column {
                line.push_str(&" ".repeat(gap));
                line.push_str(&theme::dim(&desc));
            }
        }
        lines.push(truncate_to_width(&line, width, None));
    }

    if range.start > 0 || range.end < items.len() {
        let info = format!("  ({}/{})", selected + 1, items.len());
        lines.push(truncate_to_width(&theme::dim(&info), width, None));
    }

    lines
}

#[cfg(test)]
#[path = "list_rows_tests.rs"]
mod tests;
