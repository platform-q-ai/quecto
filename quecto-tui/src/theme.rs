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

// ── Background colors for tool boxes ─────────────────────────────────────────

/// 256-color background: `\x1b[48;5;<n>m`.
fn bg256(color: u8, text: &str) -> String {
    format!("\x1b[48;5;{}m{}\x1b[0m", color, text)
}

/// Apply background color to a line, padding to full width.
///
/// Handles embedded SGR resets (`\x1b[0m`) by re-applying the background
/// color after each reset. Without this, styled text (bold, colors) that
/// contains `\x1b[0m` would kill the background mid-line, creating an
/// ugly partial-highlight effect instead of a full-width box.
pub fn apply_bg(text: &str, width: usize, bg_fn: fn(&str) -> String) -> String {
    // Extract the 256-color code from the bg_fn by calling it on empty string.
    // bg_fn("") produces "\x1b[48;5;Nm\x1b[0m" — we extract "\x1b[48;5;Nm".
    let probe = bg_fn("");
    let bg_code = if let Some(end) = probe.find('m') {
        &probe[..=end]
    } else {
        // Fallback — shouldn't happen with our bg256 functions.
        return bg_fn(&format!(
            "{}{}",
            text,
            " ".repeat(width.saturating_sub(crate::utils::visible_width(text)))
        ));
    }
    .to_string();

    // Replace all \x1b[0m resets in the text with \x1b[0m + bg re-apply.
    // This ensures the background persists through styled content.
    let patched = text.replace("\x1b[0m", &format!("\x1b[0m{}", bg_code));

    // Pad to full width.
    let vis = crate::utils::visible_width(text);
    let pad = width.saturating_sub(vis);

    format!("{}{}{}\x1b[0m", bg_code, patched, " ".repeat(pad))
}

/// Tool pending background (dark gray).
pub fn tool_pending_bg(text: &str) -> String {
    bg256(236, text)
}

/// Tool success background (dark green).
pub fn tool_success_bg(text: &str) -> String {
    bg256(22, text)
}

/// Tool error background (dark red).
pub fn tool_error_bg(text: &str) -> String {
    bg256(52, text)
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
    fn apply_bg_plain_text_pads_to_width() {
        let result = apply_bg("hello", 20, tool_success_bg);
        // Should contain the bg code for success (22).
        assert!(result.contains("\x1b[48;5;22m"));
        // Visible width should be 20 (5 chars + 15 padding).
        let vis = crate::utils::visible_width(&result);
        assert_eq!(vis, 20, "visible width should be 20, got {}", vis);
    }

    #[test]
    fn apply_bg_preserves_background_through_sgr_resets() {
        // Styled text with embedded \x1b[0m resets — this is the key bug.
        // bold("$ ls") produces "\x1b[1m$ ls\x1b[0m"
        let styled = format!(" {} ", bold("$ ls"));
        let result = apply_bg(&styled, 40, tool_success_bg);

        // The background code should appear at the start.
        assert!(
            result.starts_with("\x1b[48;5;22m"),
            "should start with success bg: {:?}",
            &result[..30.min(result.len())]
        );

        // After the embedded \x1b[0m from bold(), the bg should be re-applied
        // so that padding spaces after the text still have the background.
        // Count how many times the bg code appears — should be more than once
        // if there are embedded resets.
        let bg_code = "\x1b[48;5;22m";
        let occurrences = result.matches(bg_code).count();
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

        // The bg code should be present at the start and re-applied after resets.
        let bg_code = "\x1b[48;5;22m";
        let occurrences = result.matches(bg_code).count();
        assert!(
            occurrences >= 2,
            "bg should persist through styled elements, found {} occurrences",
            occurrences
        );
    }

    #[test]
    fn apply_bg_no_resets_in_plain_text() {
        // Plain text without any ANSI codes — bg should wrap normally.
        let result = apply_bg("plain text", 30, tool_pending_bg);
        let bg_code = "\x1b[48;5;236m";
        assert!(result.starts_with(bg_code));
        let vis = crate::utils::visible_width(&result);
        assert_eq!(vis, 30);
    }

    #[test]
    fn apply_bg_error_bg_works() {
        let result = apply_bg("error!", 20, tool_error_bg);
        assert!(result.contains("\x1b[48;5;52m"));
        let vis = crate::utils::visible_width(&result);
        assert_eq!(vis, 20);
    }

    #[test]
    fn apply_bg_empty_text_fills_width() {
        let result = apply_bg("", 10, tool_success_bg);
        let vis = crate::utils::visible_width(&result);
        assert_eq!(vis, 10, "empty text should pad to full width");
    }
}
