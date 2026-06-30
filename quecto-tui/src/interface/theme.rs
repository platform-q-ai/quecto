//! ANSI color theme — lightweight styling via escape codes.
//!
//! No external crate. Just raw SGR (Select Graphic Rendition) sequences.

/// Shared braille spinner frames, so every animated indicator (the agent
/// spinner, sub-agent rows, the idle "N working" line) cycles identically.
pub const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Static glyph marking the footer model as actively streaming. Deliberately a
/// non-spinner symbol: the footer renders without access to the animation tick,
/// so a spinner frame here would read as frozen. Frame-cycling stays owned by
/// the `Spinner` component.
pub const STREAMING_INDICATOR: &str = "●";

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

/// Reverse-video text (swap fg/bg) — used to mark the selected panel row.
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

pub fn gray(text: &str) -> String {
    fg256(245, text)
}

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
/// Modal overlay background — the terminal's DEFAULT background (`\x1b[49m`), so
/// overlays follow the active theme (light or dark) like the rest of the TUI
/// instead of painting a hardcoded fill. Modals are delineated by a box border.
pub const BG_OVERLAY: &str = "\x1b[49m";

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
///
/// Tool output carries third-party SGR sequences. Any escape whose net effect
/// leaves the background at default — a reset (`\x1b[0m`, `\x1b[m`, `\x1b[0;1m`)
/// or the default-bg code (`\x1b[49m`) — would otherwise drop the box
/// background for the rest of the line, leaving a gap. So the box bg is
/// re-asserted after each such escape (and again before the padding). SGRs that
/// *set* a background (an inner highlight like `48;2;…`) are left intact so
/// deliberate highlights survive.
fn apply_bg_code(text: &str, width: usize, bg_code: &str) -> String {
    use crate::interface::ansi::{AnsiSegment, ansi_segments};

    let mut out = String::with_capacity(text.len() + bg_code.len() * 4 + width);
    out.push_str(bg_code);

    for seg in ansi_segments(text) {
        match seg {
            AnsiSegment::Text(t) => out.push_str(t),
            AnsiSegment::Escape(esc) => {
                out.push_str(esc);
                // CSI ending in `m` (an SGR) may clear the background; re-assert
                // the box bg after one that does. OSC and other escapes never
                // touch the background, so they pass through untouched.
                if let Some(params) = esc
                    .strip_prefix("\x1b[")
                    .and_then(|rest| rest.strip_suffix('m'))
                {
                    if sgr_clears_bg(params) {
                        out.push_str(bg_code);
                    }
                }
            }
        }
    }

    let pad = width.saturating_sub(crate::interface::utils::visible_width(text));
    out.push_str(bg_code);
    out.push_str(&" ".repeat(pad));
    out.push_str("\x1b[0m");
    out
}

/// Whether an SGR parameter list leaves the background at its default — i.e. the
/// box bg must be re-asserted after it. Tracks the net effect in order: `0`/empty
/// and `49` clear it; a standard (`40`–`47`/`100`–`107`) or extended (`48;…`) bg
/// sets it. Extended-colour value params (`38`/`48` … `5;n` or `2;r;g;b`) are
/// consumed so their components (e.g. the zeros in `48;2;0;0;0`) aren't misread.
fn sgr_clears_bg(params: &str) -> bool {
    if params.is_empty() {
        return true; // `\x1b[m` == full reset
    }
    let mut bg_off = false;
    let mut it = params.split(';');
    while let Some(p) = it.next() {
        let code: u16 = if p.is_empty() {
            0
        } else {
            p.parse().unwrap_or(u16::MAX)
        };
        match code {
            0 | 49 => bg_off = true,
            40..=47 | 100..=107 => bg_off = false,
            38 | 48 => {
                bg_off = code == 38 && bg_off; // 48 sets a bg; 38 leaves bg as-is
                match it.next().map(|x| x.parse::<u16>().unwrap_or(u16::MAX)) {
                    Some(5) => {
                        it.next();
                    }
                    Some(2) => {
                        it.next();
                        it.next();
                        it.next();
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    bg_off
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
}
