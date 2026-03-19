//! ANSI color theme — lightweight styling via escape codes.
//!
//! No external crate. Just raw SGR (Select Graphic Rendition) sequences.

/// Apply an SGR code around text, with reset after.
fn styled(code: u8, text: &str) -> String {
    format!("\x1b[{}m{}\x1b[0m", code, text)
}

/// 256-color foreground: `\x1b[38;5;<n>m`.
fn fg256(color: u8, text: &str) -> String {
    format!("\x1b[38;5;{}m{}\x1b[0m", color, text)
}

/// Bold text.
pub fn bold(text: &str) -> String {
    styled(1, text)
}

/// Dim (faint) text.
pub fn dim(text: &str) -> String {
    styled(2, text)
}

/// Italic text.
pub fn italic(text: &str) -> String {
    styled(3, text)
}

/// Underline text.
pub fn underline(text: &str) -> String {
    styled(4, text)
}

/// Reverse video (swap fg/bg).
pub fn reverse(text: &str) -> String {
    styled(7, text)
}

// ── Standard colors ──────────────────────────────────────────────────────────

pub fn red(text: &str) -> String {
    styled(31, text)
}

pub fn green(text: &str) -> String {
    styled(32, text)
}

pub fn yellow(text: &str) -> String {
    styled(33, text)
}

pub fn blue(text: &str) -> String {
    styled(34, text)
}

pub fn magenta(text: &str) -> String {
    styled(35, text)
}

pub fn cyan(text: &str) -> String {
    styled(36, text)
}

pub fn white(text: &str) -> String {
    styled(37, text)
}

pub fn gray(text: &str) -> String {
    fg256(245, text)
}

// ── SGR reset ────────────────────────────────────────────────────────────────

/// Full SGR reset + OSC 8 hyperlink reset.
pub const RESET: &str = "\x1b[0m\x1b]8;;\x07";

/// SGR reset only.
pub const SGR_RESET: &str = "\x1b[0m";

// ── Semantic colors ──────────────────────────────────────────────────────────

/// Accent color (for prompts, borders, highlights).
pub fn accent(text: &str) -> String {
    cyan(text)
}

/// Muted color (for secondary information).
pub fn muted(text: &str) -> String {
    gray(text)
}

/// Success color.
pub fn success(text: &str) -> String {
    green(text)
}

/// Error color.
pub fn error(text: &str) -> String {
    red(text)
}

/// Warning color.
pub fn warning(text: &str) -> String {
    yellow(text)
}

/// Spinner frame color.
pub fn spinner(text: &str) -> String {
    cyan(text)
}

/// Tool name color.
pub fn tool_name(text: &str) -> String {
    blue(text)
}

// ── Tool output foreground color ──────────────────────────────────────────────

/// Tool output text color — matches Pi's `toolOutput` (#808080).
pub fn tool_output(text: &str) -> String {
    format!("\x1b[38;2;128;128;128m{}\x1b[0m", text)
}

// ── Background colors for tool boxes ─────────────────────────────────────────
//
// Colors match Pi's dark theme exactly (from dark.json):
//   toolPendingBg: #282832 — very dark blue-gray
//   toolSuccessBg: #283228 — very dark muted green
//   toolErrorBg:   #3c2828 — very dark muted red

/// Truecolor background: `\x1b[48;2;R;G;Bm`.
fn bg_rgb(r: u8, g: u8, b: u8, text: &str) -> String {
    format!("\x1b[48;2;{};{};{}m{}\x1b[0m", r, g, b, text)
}

/// Apply background color to a line, padding to full width.
///
/// Handles embedded SGR resets (`\x1b[0m`) from our `styled()` / `bold()` /
/// `dim()` etc. functions by re-applying the background color after each
/// reset. Without this, styled text kills the background mid-line, creating
/// an ugly partial-highlight effect instead of a full-width box.
///
/// Only handles `\x1b[0m` (full SGR reset, which all our styling helpers
/// emit) — not selective resets like `\x1b[49m`.
pub fn apply_bg(text: &str, width: usize, bg_fn: fn(&str) -> String) -> String {
    apply_bg_code(text, width, bg_code_from_fn(bg_fn))
}

/// Background ANSI codes for the three tool states (truecolor, matching Pi).
pub const BG_PENDING: &str = "\x1b[48;2;40;40;50m"; // #282832
pub const BG_SUCCESS: &str = "\x1b[48;2;40;50;40m"; // #283228
pub const BG_ERROR: &str = "\x1b[48;2;60;40;40m"; // #3c2828

/// Map a bg function pointer to its known ANSI background code constant.
fn bg_code_from_fn(bg_fn: fn(&str) -> String) -> &'static str {
    // Cast through *const () to avoid "function pointer comparison" warnings.
    let ptr = bg_fn as *const ();
    if ptr == tool_pending_bg as *const () {
        BG_PENDING
    } else if ptr == tool_success_bg as *const () {
        BG_SUCCESS
    } else if ptr == tool_error_bg as *const () {
        BG_ERROR
    } else {
        BG_PENDING
    }
}

/// Core implementation: apply a background ANSI code to text, padding to width.
fn apply_bg_code(text: &str, width: usize, bg_code: &str) -> String {
    // Build the reset-and-reapply sequence once.
    let reset_and_reapply = format!("\x1b[0m{}", bg_code);

    // Replace all \x1b[0m resets in the text with reset + bg re-apply.
    // This ensures the background persists through styled content.
    let patched = text.replace("\x1b[0m", &reset_and_reapply);

    // Pad to full width.
    let vis = crate::utils::visible_width(text);
    let pad = width.saturating_sub(vis);

    format!("{}{}{}\x1b[0m", bg_code, patched, " ".repeat(pad))
}

/// Tool pending background — #282832 (very dark blue-gray, matches Pi).
pub fn tool_pending_bg(text: &str) -> String {
    bg_rgb(40, 40, 50, text)
}

/// Tool success background — #283228 (very dark muted green, matches Pi).
pub fn tool_success_bg(text: &str) -> String {
    bg_rgb(40, 50, 40, text)
}

/// Tool error background — #3c2828 (very dark muted red, matches Pi).
pub fn tool_error_bg(text: &str) -> String {
    bg_rgb(60, 40, 40, text)
}

/// Tool title (bold, default foreground — matches Pi's toolTitle: "").
pub fn tool_title(text: &str) -> String {
    bold(text)
}

#[cfg(test)]
mod tests {
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

    // ── Tool output color ──────────────────────────────────────────

    #[test]
    fn tool_output_uses_gray() {
        let s = tool_output("test");
        // #808080 = rgb(128,128,128) truecolor
        assert!(s.contains("\x1b[38;2;128;128;128m"));
        assert!(s.contains("test"));
    }

    // ── apply_bg tests ───────────────────────────────────────────────

    #[test]
    fn apply_bg_plain_text_pads_to_width() {
        let result = apply_bg("hello", 20, tool_success_bg);
        // Should contain the truecolor bg code for success (#283228).
        assert!(result.contains(BG_SUCCESS));
        // Visible width should be 20 (5 chars + 15 padding).
        let vis = crate::utils::visible_width(&result);
        assert_eq!(vis, 20, "visible width should be 20, got {}", vis);
    }

    #[test]
    fn apply_bg_preserves_background_through_sgr_resets() {
        // Styled text with embedded \x1b[0m resets.
        // bold("$ ls") produces "\x1b[1m$ ls\x1b[0m"
        let styled = format!(" {} ", bold("$ ls"));
        let result = apply_bg(&styled, 40, tool_success_bg);

        // The background code should appear at the start.
        assert!(
            result.starts_with(BG_SUCCESS),
            "should start with success bg: {:?}",
            &result[..40.min(result.len())]
        );

        // After the embedded \x1b[0m from bold(), the bg should be re-applied.
        let occurrences = result.matches(BG_SUCCESS).count();
        assert!(
            occurrences >= 2,
            "bg should be re-applied after embedded resets, found {} occurrences in {:?}",
            occurrences,
            result
        );
    }

    #[test]
    fn apply_bg_with_multiple_styled_elements() {
        // Simulates a tool header: icon + bold title + dim args
        let content = format!(" {} {} {} ", green("✓"), bold("$ cargo test"), dim("42ms"));
        let result = apply_bg(&content, 60, tool_success_bg);
        let vis = crate::utils::visible_width(&result);
        assert_eq!(vis, 60, "visible width should be 60, got {}", vis);

        let occurrences = result.matches(BG_SUCCESS).count();
        assert!(
            occurrences >= 2,
            "bg should persist through styled elements, found {} occurrences",
            occurrences
        );
    }

    #[test]
    fn apply_bg_no_resets_in_plain_text() {
        let result = apply_bg("plain text", 30, tool_pending_bg);
        assert!(result.starts_with(BG_PENDING));
        let vis = crate::utils::visible_width(&result);
        assert_eq!(vis, 30);
    }

    #[test]
    fn apply_bg_error_bg_works() {
        let result = apply_bg("error!", 20, tool_error_bg);
        assert!(result.contains(BG_ERROR));
        let vis = crate::utils::visible_width(&result);
        assert_eq!(vis, 20);
    }

    #[test]
    fn apply_bg_empty_text_fills_width() {
        let result = apply_bg("", 10, tool_success_bg);
        let vis = crate::utils::visible_width(&result);
        assert_eq!(vis, 10, "empty text should pad to full width");
    }

    // ── Background color values match Pi's dark.json ─────────────────

    #[test]
    fn bg_pending_matches_pi() {
        // Pi dark.json: toolPendingBg = #282832 = rgb(40,40,50)
        assert_eq!(BG_PENDING, "\x1b[48;2;40;40;50m");
    }

    #[test]
    fn bg_success_matches_pi() {
        // Pi dark.json: toolSuccessBg = #283228 = rgb(40,50,40)
        assert_eq!(BG_SUCCESS, "\x1b[48;2;40;50;40m");
    }

    #[test]
    fn bg_error_matches_pi() {
        // Pi dark.json: toolErrorBg = #3c2828 = rgb(60,40,40)
        assert_eq!(BG_ERROR, "\x1b[48;2;60;40;40m");
    }
}
