use super::*;

pub(crate) fn build_model_selector_overlay(
    selector: &mut ModelSelector,
    terminal_width: usize,
    terminal_height: usize,
) -> (Vec<String>, usize) {
    build_select_overlay(terminal_width, terminal_height, |content_width| {
        selector.render(content_width)
    })
}
use crate::interface::components::model_selector::{ModelEntry, ModelSelector};
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
    let mut sel = ModelSelector::with_models(
        vec![ModelEntry {
            id: "custom/model-a".to_string(),
            provider: "Custom".to_string(),
            auth: None,
            is_current: false,
        }],
        None,
    );
    let (lines, width) = build_model_selector_overlay(&mut sel, 100, 40);
    assert_eq!(
        width, SELECTOR_MAX_PANEL_WIDTH,
        "panel should use the max width for a 100-column terminal"
    );
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
    let mut sel = ModelSelector::with_models(
        vec![ModelEntry {
            id: "custom/model-a".to_string(),
            provider: "Custom".to_string(),
            auth: None,
            is_current: false,
        }],
        None,
    );
    let (lines, width) = build_model_selector_overlay(&mut sel, 100, 40);
    for (i, line) in lines.iter().enumerate() {
        assert!(
            line.contains(theme::BG_OVERLAY),
            "line {i} should use the opaque background"
        );
        assert_eq!(
            crate::interface::utils::visible_width(line),
            width,
            "line {i} has wrong width"
        );
    }
}

#[test]
fn model_overlay_contains_selector_items() {
    let mut sel = ModelSelector::with_models(
        vec![ModelEntry {
            id: "custom/model-a".to_string(),
            provider: "Custom".to_string(),
            auth: None,
            is_current: false,
        }],
        None,
    );
    let (lines, _) = build_model_selector_overlay(&mut sel, 100, 40);
    let joined = lines.join("\n");
    assert!(joined.contains("Select Model"), "should contain title");
    assert!(
        joined.contains("custom/model-a"),
        "should contain the custom model"
    );
}

#[test]
fn model_overlay_width_is_bounded() {
    let mut sel = ModelSelector::with_models(Vec::new(), None);
    let (_, width) = build_model_selector_overlay(&mut sel, 200, 40);
    assert!(
        width <= SELECTOR_MAX_PANEL_WIDTH,
        "overlay width {width} should not exceed max {SELECTOR_MAX_PANEL_WIDTH}"
    );
    let (_, width) = build_model_selector_overlay(&mut sel, 2, 40);
    assert_eq!(width, 1, "panel width should clamp to 1 for tiny terminals");
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
