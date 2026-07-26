//! Shared modal rendering for `SelectList` overlays.

use std::time::Duration;

use crate::components::component::Component;
use crate::components::select_list::SelectList;
use crate::interface::theme;

/// Time window for idle double-Escape to open the rewind selector.
pub const DOUBLE_ESC_WINDOW: Duration = Duration::from_millis(750);

/// Maximum width of the selector modal, including its box border.
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

pub(crate) fn build_select_list_overlay(
    title: &str,
    footer: &str,
    selector: &mut SelectList,
    terminal_width: usize,
    terminal_height: usize,
) -> (Vec<String>, usize) {
    build_select_overlay(terminal_width, terminal_height, |content_width| {
        let mut content_lines = vec![theme::bold(title)];
        content_lines.extend(selector.render(content_width));
        content_lines.push(theme::dim(footer));
        content_lines
    })
}

pub(crate) fn build_resume_selector_overlay(
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

pub(crate) fn build_rewind_selector_overlay(
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

pub(crate) fn build_select_overlay(
    terminal_width: usize,
    terminal_height: usize,
    render_content: impl FnOnce(usize) -> Vec<String>,
) -> (Vec<String>, usize) {
    let panel_width = terminal_width
        .saturating_sub(4)
        .clamp(1, SELECTOR_MAX_PANEL_WIDTH);
    // A box border ("│ … │") needs the two border columns plus a space of
    // padding on each side; narrower than that, fall back to plain padded rows.
    let bordered = panel_width >= 6;
    let content_width = if bordered {
        panel_width - 4
    } else {
        panel_width
    }
    .max(1);

    let content_lines = render_content(content_width);
    let max_height = terminal_height.saturating_sub(4).max(1);

    // Every row runs through `apply_overlay_bg`, which sets the terminal's
    // DEFAULT background — so the modal follows the active theme like the rest of
    // the TUI. A box border (default foreground) gives it structure without a
    // hardcoded opaque fill.
    let mut overlay_lines = Vec::new();
    if bordered {
        let horizontal = "─".repeat(panel_width - 2);
        overlay_lines.push(theme::apply_overlay_bg(
            &format!("┌{horizontal}┐"),
            panel_width,
        ));
        for line in content_lines {
            let padded = pad_ansi_to_width(&line, content_width);
            overlay_lines.push(theme::apply_overlay_bg(
                &format!("│ {padded} │"),
                panel_width,
            ));
        }
        overlay_lines.push(theme::apply_overlay_bg(
            &format!("└{horizontal}┘"),
            panel_width,
        ));
    } else {
        for line in content_lines {
            let padded = pad_ansi_to_width(&line, content_width);
            overlay_lines.push(theme::apply_overlay_bg(&padded, panel_width));
        }
    }
    overlay_lines.truncate(max_height);

    (overlay_lines, panel_width)
}

#[cfg(test)]
#[path = "select_overlay_tests.rs"]
mod tests;
