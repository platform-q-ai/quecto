//! Shared modal rendering for `SelectList` overlays.

use std::time::Duration;

use crate::interface::component::Component;
use crate::interface::components::model_selector::ModelSelector;
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
    let mut content_lines = vec![theme::bold(title)];
    content_lines.extend(selector.render(terminal_width));
    content_lines.push(theme::dim(footer));
    let render_fn = |_content_width: usize| content_lines.clone();
    build_select_overlay(terminal_width, terminal_height, render_fn)
}

fn build_select_overlay(
    terminal_width: usize,
    terminal_height: usize,
    render_content: impl FnOnce(usize) -> Vec<String>,
) -> (Vec<String>, usize) {
    let panel_width = terminal_width
        .saturating_sub(4)
        .clamp(1, SELECTOR_MAX_PANEL_WIDTH);
    let border_width = SELECTOR_BORDER_WIDTH.min(panel_width.saturating_sub(20) / 2);
    let content_width = panel_width.saturating_sub(border_width * 2).max(1);
    let side_padding = " ".repeat(border_width);

    let content_lines = render_content(content_width);
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

pub fn build_model_selector_overlay(
    selector: &mut ModelSelector,
    terminal_width: usize,
    terminal_height: usize,
) -> (Vec<String>, usize) {
    build_select_overlay(terminal_width, terminal_height, |content_width| {
        selector.render(content_width)
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::components::model_selector::ModelSelector;
    use crate::interface::components::select_list::{SelectItem, SelectList};

    fn sample_selector() -> SelectList {
        SelectList::new(
            vec![
                SelectItem {
                    label: "alpha".into(),
                    value: "0".into(),
                    description: Some("2 messages".into()),
                },
                SelectItem {
                    label: "beta".into(),
                    value: "1".into(),
                    description: None,
                },
            ],
            10,
        )
    }

    // ── resume selector overlay ─────────────────────────────────────

    #[test]
    fn resume_overlay_has_title_and_footer() {
        let mut sel = sample_selector();
        let (lines, _) = build_resume_selector_overlay(&mut sel, 100, 40);
        let joined = lines.join("\n");
        assert!(joined.contains("Resume session"), "should contain title");
        assert!(
            joined.contains("Enter resume"),
            "should contain footer hint"
        );
    }

    #[test]
    fn resume_overlay_contains_selector_items() {
        let mut sel = sample_selector();
        let (lines, _) = build_resume_selector_overlay(&mut sel, 100, 40);
        let joined = lines.join("\n");
        assert!(joined.contains("alpha"), "should render first item");
        assert!(joined.contains("beta"), "should render second item");
    }

    #[test]
    fn resume_overlay_width_is_bounded() {
        let mut sel = sample_selector();
        let (_, width) = build_resume_selector_overlay(&mut sel, 200, 40);
        assert!(
            width <= SELECTOR_MAX_PANEL_WIDTH,
            "overlay width {width} should not exceed max {SELECTOR_MAX_PANEL_WIDTH}"
        );
    }

    #[test]
    fn resume_overlay_clamps_to_terminal_width() {
        let mut sel = sample_selector();
        let (_, width) = build_resume_selector_overlay(&mut sel, 20, 40);
        assert!(
            width <= 20,
            "overlay width {width} should not exceed terminal width 20"
        );
    }

    #[test]
    fn resume_overlay_lines_do_not_exceed_max_height() {
        let mut sel = sample_selector();
        let (lines, _) = build_resume_selector_overlay(&mut sel, 100, 10);
        assert!(
            lines.len() <= 6,
            "overlay should be clamped to terminal_height - 4 = 6"
        );
    }

    // ── model selector overlay ────────────────────────────────────

    #[test]
    fn model_overlay_lines_span_full_width() {
        let mut sel = ModelSelector::new(Some("anthropic-api/claude-fable-5"));
        let (lines, width) = build_model_selector_overlay(&mut sel, 100, 40);
        for (i, line) in lines.iter().enumerate() {
            assert_eq!(
                crate::interface::utils::visible_width(line),
                width,
                "line {i} has wrong width"
            );
        }
    }

    #[test]
    fn model_overlay_uses_opaque_background() {
        let mut sel = ModelSelector::new(Some("anthropic-api/claude-fable-5"));
        let (lines, _) = build_model_selector_overlay(&mut sel, 100, 40);
        assert!(
            lines.iter().all(|line| line.contains(theme::BG_OVERLAY)),
            "every model overlay line should use the opaque background"
        );
    }

    #[test]
    fn model_overlay_contains_selector_items() {
        let mut sel = ModelSelector::new(None);
        let (lines, _) = build_model_selector_overlay(&mut sel, 100, 40);
        let joined = lines.join("\n");
        assert!(joined.contains("Select Model"), "should contain title");
        assert!(joined.contains("claude-fable-5"), "should contain a model");
    }

    #[test]
    fn model_overlay_width_is_bounded() {
        let mut sel = ModelSelector::new(None);
        let (_, width) = build_model_selector_overlay(&mut sel, 200, 40);
        assert!(
            width <= SELECTOR_MAX_PANEL_WIDTH,
            "overlay width {width} should not exceed max {SELECTOR_MAX_PANEL_WIDTH}"
        );
    }

    // ── rewind selector overlay ─────────────────────────────────────

    #[test]
    fn rewind_overlay_has_title_and_footer() {
        let mut sel = sample_selector();
        let (lines, _) = build_rewind_selector_overlay(&mut sel, 100, 40);
        let joined = lines.join("\n");
        assert!(joined.contains("Go back to"), "should contain title");
        assert!(
            joined.contains("Enter select"),
            "should contain footer hint"
        );
    }

    #[test]
    fn rewind_overlay_contains_items() {
        let mut sel = sample_selector();
        let (lines, _) = build_rewind_selector_overlay(&mut sel, 100, 40);
        let joined = lines.join("\n");
        assert!(joined.contains("alpha"));
        assert!(joined.contains("beta"));
    }

    // ── shared behavior ─────────────────────────────────────────────

    #[test]
    fn overlay_uses_opaque_background() {
        let mut sel = sample_selector();
        let (lines, _) = build_resume_selector_overlay(&mut sel, 100, 40);
        assert!(
            lines.iter().any(|l| l.contains(theme::BG_OVERLAY)),
            "at least one line should use the opaque background"
        );
    }

    #[test]
    fn overlay_lines_are_uniform_width() {
        let mut sel = sample_selector();
        let (lines, width) = build_resume_selector_overlay(&mut sel, 100, 40);
        for (i, line) in lines.iter().enumerate() {
            let vis = crate::interface::utils::visible_width(line);
            assert_eq!(
                vis, width,
                "line {i} has visible width {vis} but overlay width is {width}"
            );
        }
    }

    #[test]
    fn overlay_with_empty_selector_still_has_title() {
        let mut sel = SelectList::new(vec![], 10);
        let (lines, _) = build_resume_selector_overlay(&mut sel, 100, 40);
        let joined = lines.join("\n");
        assert!(joined.contains("Resume session"));
    }

    #[test]
    fn overlay_with_single_item() {
        let mut sel = SelectList::new(
            vec![SelectItem {
                label: "only".into(),
                value: "0".into(),
                description: None,
            }],
            10,
        );
        let (lines, _) = build_resume_selector_overlay(&mut sel, 100, 40);
        let joined = lines.join("\n");
        assert!(joined.contains("only"));
    }

    #[test]
    fn double_esc_window_is_reasonable() {
        assert!(DOUBLE_ESC_WINDOW.as_millis() >= 200);
        assert!(DOUBLE_ESC_WINDOW.as_millis() <= 2000);
    }

    // ── pad_ansi_to_width ───────────────────────────────────────────

    #[test]
    fn pad_short_text_to_width() {
        let result = pad_ansi_to_width("hi", 10);
        assert_eq!(crate::interface::utils::visible_width(&result), 10);
    }

    #[test]
    fn pad_exact_width_unchanged() {
        let result = pad_ansi_to_width("hello", 5);
        assert_eq!(crate::interface::utils::visible_width(&result), 5);
    }

    #[test]
    fn pad_truncates_overlong_text() {
        let result = pad_ansi_to_width("hello world", 5);
        assert!(
            crate::interface::utils::visible_width(&result) <= 5,
            "overlong text should be truncated"
        );
    }

    #[test]
    fn pad_preserves_ansi_escapes() {
        let input = "\x1b[31mred\x1b[0m";
        let result = pad_ansi_to_width(input, 10);
        assert!(result.contains("\x1b[31m"), "ANSI should be preserved");
    }
}
