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

/// Tool output text color — matches Quecto's `toolOutput` (#808080).
pub fn tool_output(text: &str) -> String {
    format!("\x1b[38;2;128;128;128m{}\x1b[0m", text)
}

// ── Background colors for tool boxes ─────────────────────────────────────────
//
// Colors match Quecto's dark theme exactly (from dark.json):
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

/// Background ANSI codes for the three tool states (truecolor, matching Quecto).
pub const BG_PENDING: &str = "\x1b[48;2;40;40;50m"; // #282832
pub const BG_SUCCESS: &str = "\x1b[48;2;40;50;40m"; // #283228
pub const BG_ERROR: &str = "\x1b[48;2;60;40;40m"; // #3c2828
/// Opaque modal overlay background — black for maximum contrast over chat text.
pub const BG_OVERLAY: &str = "\x1b[48;2;0;0;0m";

/// Extract the background ANSI escape code from a bg function.
///
/// Probes by calling `bg_fn("")` which produces `"\x1b[48;5;Nm\x1b[0m"`,
/// then extracts everything up to and including the first `m`.
fn bg_code_from_fn(bg_fn: fn(&str) -> String) -> &'static str {
    // Use function pointer identity to map to known constants.
    // Cast through *const () to avoid "function pointer comparison" warnings.
    let ptr = bg_fn as *const ();
    if ptr == tool_pending_bg as *const () {
        BG_PENDING
    } else if ptr == tool_success_bg as *const () {
        BG_SUCCESS
    } else if ptr == tool_error_bg as *const () {
        BG_ERROR
    } else {
        // Unknown bg function — fall back to pending.
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
    let vis = crate::interface::utils::visible_width(text);
    let pad = width.saturating_sub(vis);

    format!("{}{}{}\x1b[0m", bg_code, patched, " ".repeat(pad))
}

/// Tool pending background — #282832 (very dark blue-gray, matches Quecto).
pub fn tool_pending_bg(text: &str) -> String {
    bg_rgb(40, 40, 50, text)
}

/// Tool success background — #283228 (very dark muted green, matches Quecto).
pub fn tool_success_bg(text: &str) -> String {
    bg_rgb(40, 50, 40, text)
}

/// Tool error background — #3c2828 (very dark muted red, matches Quecto).
pub fn tool_error_bg(text: &str) -> String {
    bg_rgb(60, 40, 40, text)
}

/// Apply the opaque modal overlay background to a line, padding to full width.
pub fn apply_overlay_bg(text: &str, width: usize) -> String {
    apply_bg_code(text, width, BG_OVERLAY)
}

/// Tool title (bold, used inside tool boxes).
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

    #[test]
    fn bg_colors_match_quecto_dark_theme() {
        assert_eq!(BG_PENDING, "\x1b[48;2;40;40;50m");
        assert_eq!(BG_SUCCESS, "\x1b[48;2;40;50;40m");
        assert_eq!(BG_ERROR, "\x1b[48;2;60;40;40m");
    }
}
