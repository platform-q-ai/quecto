use super::*;

#[test]
fn bold_wraps_with_sgr() {
    let s = bold("hello");
    assert!(s.starts_with("\x1b[1m"));
    assert!(s.ends_with("\x1b[0m"));
    assert!(s.contains("hello"));
}

#[test]
fn red_applies_color() {
    let s = red("err");
    assert!(s.contains("\x1b[31m"));
    assert!(s.contains("err"));
}

#[test]
fn gray_uses_256_color() {
    let s = gray("muted");
    assert!(s.contains("\x1b[38;5;245m"));
}

// ── apply_bg tests ───────────────────────────────────────────────

#[test]
fn tool_output_uses_gray() {
    let s = tool_output("test");
    assert!(s.contains("\x1b[38;2;128;128;128m"));
}

#[test]
fn apply_bg_plain_text_pads_to_width() {
    let result = apply_bg("hello", 20, tool_success_bg);
    assert!(result.contains(BG_SUCCESS));
    let vis = crate::interface::utils::visible_width(&result);
    assert_eq!(vis, 20);
}

#[test]
fn apply_bg_preserves_background_through_sgr_resets() {
    let styled = format!(" {} ", bold("$ ls"));
    let result = apply_bg(&styled, 40, tool_success_bg);
    assert!(result.starts_with(BG_SUCCESS));
    let occurrences = result.matches(BG_SUCCESS).count();
    assert!(occurrences >= 2);
}

#[test]
fn apply_bg_with_multiple_styled_elements() {
    let content = format!(" {} {} {} ", green("✓"), bold("$ cargo test"), dim("42ms"));
    let result = apply_bg(&content, 60, tool_success_bg);
    let vis = crate::interface::utils::visible_width(&result);
    assert_eq!(vis, 60);
    let occurrences = result.matches(BG_SUCCESS).count();
    assert!(occurrences >= 2);
}

#[test]
fn apply_bg_no_resets_in_plain_text() {
    let result = apply_bg("plain text", 30, tool_pending_bg);
    assert!(result.starts_with(BG_PENDING));
    assert_eq!(crate::interface::utils::visible_width(&result), 30);
}

#[test]
fn apply_bg_error_bg_works() {
    let result = apply_bg("error!", 20, tool_error_bg);
    assert!(result.contains(BG_ERROR));
    assert_eq!(crate::interface::utils::visible_width(&result), 20);
}

#[test]
fn apply_bg_empty_text_fills_width() {
    let result = apply_bg("", 10, tool_success_bg);
    assert_eq!(crate::interface::utils::visible_width(&result), 10);
}

// ── gap fix: re-assert bg after ANY bg-clearing escape, not just \x1b[0m ──

#[test]
fn apply_bg_reasserts_after_bare_reset() {
    // Foreign tool output often resets with `\x1b[m` (empty params), which a
    // literal `\x1b[0m` replace misses — leaving a background gap.
    let result = apply_bg("a\x1b[mb", 10, tool_success_bg);
    assert!(
        result.contains(&format!("\x1b[m{BG_SUCCESS}")),
        "bare reset must re-assert the box bg: {result:?}"
    );
}

#[test]
fn apply_bg_reasserts_after_default_bg_49() {
    // `\x1b[49m` sets the default background — must re-assert the box bg.
    let result = apply_bg("x\x1b[49my", 10, tool_success_bg);
    assert!(
        result.contains(&format!("\x1b[49m{BG_SUCCESS}")),
        "default-bg (49) must re-assert the box bg: {result:?}"
    );
}

#[test]
fn apply_bg_reasserts_after_compound_reset() {
    // `\x1b[0;1m` (reset bundled with bold) also clears bg.
    let result = apply_bg("p\x1b[0;1mq", 10, tool_success_bg);
    assert!(
        result.contains(&format!("\x1b[0;1m{BG_SUCCESS}")),
        "compound reset must re-assert the box bg: {result:?}"
    );
}

#[test]
fn apply_bg_preserves_inner_background_highlight() {
    // A deliberate inner background (e.g. a diff-context highlight) must NOT
    // be immediately overridden by the box bg.
    let grey = "\x1b[48;2;80;80;80m";
    let result = apply_bg(&format!("{grey}hi\x1b[0m"), 10, tool_success_bg);
    assert!(
        !result.contains(&format!("{grey}{BG_SUCCESS}")),
        "inner highlight must survive: {result:?}"
    );
}

#[test]
fn apply_bg_keeps_truecolor_bg_with_zero_component() {
    // `48;2;0;0;0` is a black background, not a reset — the zero components
    // must not be misread as a `0` reset and clobbered.
    let black = "\x1b[48;2;0;0;0m";
    let result = apply_bg(&format!("{black}z\x1b[0m"), 10, tool_success_bg);
    assert!(
        !result.contains(&format!("{black}{BG_SUCCESS}")),
        "truecolor bg with zero components must survive: {result:?}"
    );
}

#[test]
fn text_style_helpers_emit_expected_sgr_codes() {
    let cases = [
        (italic("i"), "\x1b[3mi\x1b[0m"),
        (underline("u"), "\x1b[4mu\x1b[0m"),
        (reverse("r"), "\x1b[7mr\x1b[0m"),
        (magenta("m"), "\x1b[35mm\x1b[0m"),
    ];

    for (actual, expected) in cases {
        assert_eq!(actual, expected);
    }
}

#[test]
fn semantic_helpers_delegate_to_expected_colours() {
    assert_eq!(accent("x"), cyan("x"));
    assert_eq!(muted("x"), gray("x"));
    assert_eq!(success("x"), green("x"));
    assert_eq!(error("x"), red("x"));
    assert_eq!(warning("x"), yellow("x"));
    assert_eq!(spinner("x"), cyan("x"));
    assert_eq!(tool_name("x"), blue("x"));
    assert_eq!(tool_title("x"), bold("x"));
}

#[test]
fn background_helpers_and_overlay_emit_expected_codes() {
    assert_eq!(tool_pending_bg("p"), format!("{BG_PENDING}p\x1b[0m"));
    assert_eq!(tool_success_bg("s"), format!("{BG_SUCCESS}s\x1b[0m"));
    assert_eq!(tool_error_bg("e"), format!("{BG_ERROR}e\x1b[0m"));

    let overlay = apply_overlay_bg("hi", 5);
    assert!(
        overlay.starts_with(BG_OVERLAY),
        "overlay should use default-bg code: {overlay:?}"
    );
    assert_eq!(crate::interface::utils::visible_width(&overlay), 5);
}
