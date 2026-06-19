//! Shared modal rendering for `SelectList` overlays.

use std::time::Duration;

use crate::interface::component::Component;
use crate::interface::components::select_list::SelectList;
use crate::interface::theme;

/// Time window for idle double-Escape to open the rewind selector.
pub const DOUBLE_ESC_WINDOW: Duration = Duration::from_millis(750);

/// Width of the opaque padding around selector content.
const SELECTOR_BORDER_WIDTH: usize = 6;
/// Maximum width of selector modal, including opaque padding.
const SELECTOR_MAX_PANEL_WIDTH: usize = 88;

fn pad_ansi_to_width(text: &str, width: usize) -> String {
    let truncated = crate::interface::utils::truncate_to_width(text, width, Some("…"));
    let visible = crate::interface::utils::visible_width(&truncated);
    if visible >= width {
        truncated
    } else {
        format!("{}{}", truncated, " ".repeat(width - visible))
    }
}

fn build_select_list_overlay(
    title: &str,
    footer: &str,
    selector: &mut SelectList,
    terminal_width: usize,
    terminal_height: usize,
) -> (Vec<String>, usize) {
    let panel_width = terminal_width
        .saturating_sub(4)
        .clamp(1, SELECTOR_MAX_PANEL_WIDTH);
    let border_width = SELECTOR_BORDER_WIDTH.min(panel_width.saturating_sub(20) / 2);
    let content_width = panel_width.saturating_sub(border_width * 2).max(1);
    let side_padding = " ".repeat(border_width);

    let mut content_lines = vec![theme::bold(title)];
    content_lines.extend(selector.render(content_width));
    content_lines.push(theme::dim(footer));

    let max_height = terminal_height.saturating_sub(4).max(1);
    let vertical_border = border_width.min(max_height.saturating_sub(1) / 2);
    let blank = theme::apply_overlay_bg("", panel_width);

    let mut overlay_lines = Vec::new();
    overlay_lines.extend(std::iter::repeat_n(blank.clone(), vertical_border));
    for line in content_lines {
        let padded = pad_ansi_to_width(&line, content_width);
        overlay_lines.push(theme::apply_overlay_bg(
            &format!("{side_padding}{padded}{side_padding}"),
            panel_width,
        ));
    }
    overlay_lines.extend(std::iter::repeat_n(blank, vertical_border));
    overlay_lines.truncate(max_height);

    (overlay_lines, panel_width)
}

pub fn build_resume_selector_overlay(
    selector: &mut SelectList,
    terminal_width: usize,
    terminal_height: usize,
) -> (Vec<String>, usize) {
    build_select_list_overlay(
        "Resume session",
        "Enter resume · Esc cancel",
        selector,
        terminal_width,
        terminal_height,
    )
}

pub fn build_rewind_selector_overlay(
    selector: &mut SelectList,
    terminal_width: usize,
    terminal_height: usize,
) -> (Vec<String>, usize) {
    build_select_list_overlay(
        "Go back to…",
        "Enter select · Esc cancel",
        selector,
        terminal_width,
        terminal_height,
    )
}
