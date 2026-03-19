//! Overlay system — render components on top of existing content.
//!
//! Overlays are composited into the base content lines by splicing overlay
//! content at a specific row/col position. Multiple overlays stack; the
//! topmost visible overlay captures keyboard focus.

use crate::component::Component;
use crate::utils::visible_width;

/// Anchor position for overlay placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    Center,
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

/// Options for overlay positioning and sizing.
#[derive(Debug, Clone)]
pub struct OverlayOptions {
    /// Anchor point for positioning (default: Center).
    pub anchor: Anchor,
    /// Fixed width (None = use component's rendered width).
    pub width: Option<usize>,
    /// Maximum height (None = no limit).
    pub max_height: Option<usize>,
    /// Offset from anchor position.
    pub offset_x: i32,
    pub offset_y: i32,
    /// Margin from terminal edges.
    pub margin: usize,
}

impl Default for OverlayOptions {
    fn default() -> Self {
        Self {
            anchor: Anchor::Center,
            width: None,
            max_height: None,
            offset_x: 0,
            offset_y: 0,
            margin: 1,
        }
    }
}

/// A single overlay entry in the stack.
pub struct OverlayEntry {
    pub component: Box<dyn Component>,
    pub options: OverlayOptions,
    pub hidden: bool,
    /// Focus order — higher values are rendered on top.
    pub focus_order: u64,
}

/// Manages a stack of overlays.
pub struct OverlayStack {
    entries: Vec<OverlayEntry>,
    next_order: u64,
}

impl OverlayStack {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_order: 0,
        }
    }

    /// Push an overlay onto the stack. Returns its index.
    pub fn push(&mut self, component: Box<dyn Component>, options: OverlayOptions) -> usize {
        let idx = self.entries.len();
        self.entries.push(OverlayEntry {
            component,
            options,
            hidden: false,
            focus_order: self.next_order,
        });
        self.next_order += 1;
        idx
    }

    /// Remove an overlay by index.
    pub fn remove(&mut self, index: usize) {
        if index < self.entries.len() {
            self.entries.remove(index);
        }
    }

    /// Pop the topmost overlay.
    pub fn pop(&mut self) -> Option<OverlayEntry> {
        self.entries.pop()
    }

    /// Hide/show an overlay by index.
    pub fn set_hidden(&mut self, index: usize, hidden: bool) {
        if let Some(entry) = self.entries.get_mut(index) {
            entry.hidden = hidden;
        }
    }

    /// Whether any overlays are visible.
    pub fn has_visible(&self) -> bool {
        self.entries.iter().any(|e| !e.hidden)
    }

    /// Get a mutable reference to the topmost visible overlay entry.
    pub fn topmost_entry_mut(&mut self) -> Option<&mut OverlayEntry> {
        self.entries.iter_mut().rev().find(|e| !e.hidden)
    }

    /// Number of overlays in the stack.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Composite all visible overlays into the base content lines.
    ///
    /// Overlays are rendered in focus_order (lower = behind, higher = on top).
    /// Each overlay's content is spliced into the base lines at its computed
    /// row/col position.
    pub fn composite(
        &mut self,
        base_lines: &mut Vec<String>,
        term_width: usize,
        term_height: usize,
    ) {
        // Sort by focus_order (lower first = rendered behind).
        let mut indices: Vec<usize> = (0..self.entries.len())
            .filter(|&i| !self.entries[i].hidden)
            .collect();
        indices.sort_by_key(|&i| self.entries[i].focus_order);

        for idx in indices {
            let entry = &mut self.entries[idx];
            let opts = &entry.options;

            let overlay_width = opts
                .width
                .unwrap_or(term_width.saturating_sub(opts.margin * 2));
            let overlay_width = overlay_width.min(term_width.saturating_sub(opts.margin * 2));

            let mut overlay_lines = entry.component.render(overlay_width);

            // Apply max_height.
            if let Some(max_h) = opts.max_height {
                overlay_lines.truncate(max_h);
            }

            let overlay_height = overlay_lines.len();
            if overlay_height == 0 {
                continue;
            }

            // Compute position.
            let avail_h = term_height.saturating_sub(opts.margin * 2);
            let avail_w = term_width.saturating_sub(opts.margin * 2);

            let (row, col) = resolve_position(
                opts.anchor,
                overlay_width,
                overlay_height,
                avail_w,
                avail_h,
                opts.margin,
                opts.offset_x,
                opts.offset_y,
            );

            // Ensure base_lines is tall enough.
            while base_lines.len() < row + overlay_height {
                base_lines.push(String::new());
            }

            // Splice overlay lines into base.
            for (i, overlay_line) in overlay_lines.iter().enumerate() {
                let target_row = row + i;
                if target_row < base_lines.len() {
                    base_lines[target_row] = splice_line(
                        &base_lines[target_row],
                        overlay_line,
                        col,
                        overlay_width,
                        term_width,
                    );
                }
            }
        }
    }
}

/// Resolve overlay position from anchor, dimensions, and offsets.
fn resolve_position(
    anchor: Anchor,
    width: usize,
    height: usize,
    avail_w: usize,
    avail_h: usize,
    margin: usize,
    offset_x: i32,
    offset_y: i32,
) -> (usize, usize) {
    let base_row = match anchor {
        Anchor::TopLeft | Anchor::TopCenter | Anchor::TopRight => margin,
        Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight => {
            margin + avail_h.saturating_sub(height)
        }
        Anchor::Center => margin + avail_h.saturating_sub(height) / 2,
    };

    let base_col = match anchor {
        Anchor::TopLeft | Anchor::BottomLeft => margin,
        Anchor::TopRight | Anchor::BottomRight => margin + avail_w.saturating_sub(width),
        Anchor::TopCenter | Anchor::BottomCenter | Anchor::Center => {
            margin + avail_w.saturating_sub(width) / 2
        }
    };

    let row = (base_row as i32 + offset_y).max(0) as usize;
    let col = (base_col as i32 + offset_x).max(0) as usize;

    (row, col)
}

/// Splice overlay content into a base line at the given column.
///
/// ANSI-aware: properly resets attributes at splice boundaries.
pub fn splice_line(
    base: &str,
    overlay: &str,
    start_col: usize,
    overlay_width: usize,
    total_width: usize,
) -> String {
    let base_width = visible_width(base);

    // Build: [before][overlay][after]
    let before = if start_col > 0 {
        if base_width >= start_col {
            // Take the first start_col visible characters from base.
            take_visible_chars(base, start_col)
        } else {
            // Base is shorter than start_col — pad with spaces.
            let mut s = base.to_string();
            let pad = start_col - base_width;
            s.push_str(&" ".repeat(pad));
            s
        }
    } else {
        String::new()
    };

    let after_start = start_col + overlay_width;
    let after = if after_start < total_width && after_start < base_width {
        skip_visible_chars(base, after_start)
    } else {
        String::new()
    };

    format!("{}\x1b[0m{}\x1b[0m{}", before, overlay, after)
}

/// Take the first `n` visible characters from a string (ANSI-aware).
fn take_visible_chars(s: &str, n: usize) -> String {
    let mut result = String::new();
    let mut width = 0;
    let mut in_escape = false;

    for ch in s.chars() {
        if in_escape {
            result.push(ch);
            if ch.is_ascii_alphabetic() || ch == '~' {
                in_escape = false;
            }
            continue;
        }
        if ch == '\x1b' {
            result.push(ch);
            in_escape = true;
            continue;
        }
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + cw > n {
            break;
        }
        result.push(ch);
        width += cw;
    }

    // Pad if we didn't reach n.
    while width < n {
        result.push(' ');
        width += 1;
    }

    result
}

/// Skip the first `n` visible characters and return the rest (ANSI-aware).
fn skip_visible_chars(s: &str, n: usize) -> String {
    let mut width = 0;
    let mut in_escape = false;
    let mut byte_offset = 0;

    for ch in s.chars() {
        if in_escape {
            byte_offset += ch.len_utf8();
            if ch.is_ascii_alphabetic() || ch == '~' {
                in_escape = false;
            }
            continue;
        }
        if ch == '\x1b' {
            byte_offset += ch.len_utf8();
            in_escape = true;
            continue;
        }
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + cw > n {
            break;
        }
        width += cw;
        byte_offset += ch.len_utf8();
    }

    s[byte_offset..].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::text::Text;

    #[test]
    fn overlay_composites_at_position() {
        let mut stack = OverlayStack::new();
        let opts = OverlayOptions {
            anchor: Anchor::TopLeft,
            margin: 0,
            offset_x: 5,
            offset_y: 1,
            width: Some(7),
            ..Default::default()
        };
        stack.push(Box::new(Text::new("OVERLAY")), opts);

        let mut base = vec![
            "line1".to_string(),
            "line2".to_string(),
            "line3".to_string(),
            "line4".to_string(),
        ];
        stack.composite(&mut base, 40, 4);

        // Line 1 should contain OVERLAY at col 5.
        assert!(
            base[1].contains("OVERLAY"),
            "line 1 should contain OVERLAY: {}",
            base[1]
        );
        // Line 0 should be untouched.
        assert!(base[0].contains("line1"), "line 0 should still have line1");
    }

    #[test]
    fn centered_overlay() {
        let mut stack = OverlayStack::new();
        let opts = OverlayOptions {
            anchor: Anchor::Center,
            width: Some(10),
            margin: 0,
            ..Default::default()
        };
        stack.push(Box::new(Text::new("centered")), opts);

        let mut base: Vec<String> = (0..10).map(|i| format!("line{}", i)).collect();
        stack.composite(&mut base, 40, 10);

        // "centered" should appear somewhere in the middle rows.
        let found = base.iter().any(|l| l.contains("centered"));
        assert!(found, "should contain centered overlay: {:?}", base);
    }

    #[test]
    fn hidden_overlay_not_composited() {
        let mut stack = OverlayStack::new();
        let opts = OverlayOptions::default();
        let idx = stack.push(Box::new(Text::new("hidden")), opts);
        stack.set_hidden(idx, true);

        let mut base = vec!["base".to_string()];
        stack.composite(&mut base, 40, 1);

        assert!(!base[0].contains("hidden"));
    }

    #[test]
    fn topmost_gets_focus() {
        let mut stack = OverlayStack::new();
        stack.push(Box::new(Text::new("first")), OverlayOptions::default());
        stack.push(Box::new(Text::new("second")), OverlayOptions::default());

        assert!(stack.has_visible());
        assert_eq!(stack.len(), 2);
    }

    #[test]
    fn pop_removes_topmost() {
        let mut stack = OverlayStack::new();
        stack.push(Box::new(Text::new("a")), OverlayOptions::default());
        stack.push(Box::new(Text::new("b")), OverlayOptions::default());
        stack.pop();
        assert_eq!(stack.len(), 1);
    }

    #[test]
    fn empty_stack() {
        let stack = OverlayStack::new();
        assert!(stack.is_empty());
        assert!(!stack.has_visible());
    }

    #[test]
    fn splice_line_basic() {
        let result = splice_line("AAAAAAAAAA", "XX", 3, 2, 10);
        let plain: String = result.chars().filter(|c| !c.is_control()).collect();
        assert!(plain.contains("XX"), "should contain overlay: {}", plain);
    }

    #[test]
    fn take_visible_chars_basic() {
        assert_eq!(take_visible_chars("hello world", 5), "hello");
    }

    #[test]
    fn take_visible_chars_with_ansi() {
        let s = "\x1b[31mhello\x1b[0m world";
        let result = take_visible_chars(s, 5);
        assert!(result.contains("hello"));
    }

    #[test]
    fn skip_visible_chars_basic() {
        assert_eq!(skip_visible_chars("hello world", 6), "world");
    }
}
