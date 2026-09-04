use crate::components::theme;

/// Format an elapsed duration as `m:ss` (or `h:mm:ss` past an hour) for the
/// sub-agent-first panel's per-row timers (#820).
pub(super) fn fmt_mss(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
    } else {
        format!("{}:{:02}", secs / 60, secs % 60)
    }
}

/// Per-step workflow bar beneath an agent's name (`=` done, `>` position, `.` remaining), one
/// cell per step up to `MAX_CELLS`, else proportional. Column 0 is always blank
/// so the selection (`▌`) stays one line tall; the tree stalk continues down
/// through the bar via the agent's continuation prefix.
pub(super) fn panel_bar_line(prefix: &str, done: u32, total: u32, width: usize) -> String {
    use crate::components::utils::visible_width;
    const MAX_CELLS: usize = 20;
    let cont = bar_continuation(prefix);
    let cont_vis = visible_width(&cont);
    // Reserve column 0 (blank) + a 1-col right gutter, mirroring the name row (#875).
    let avail = width.saturating_sub(2 + cont_vis);
    let cells = (total as usize).min(MAX_CELLS).min(avail).max(1);
    let filled = ((done as usize) * cells / (total.max(1) as usize)).min(cells);
    let remaining = cells - filled;
    let marker = if remaining > 0 {
        theme::yellow(">")
    } else {
        String::new()
    };
    let bar = format!(
        "{}{}{}",
        theme::green(&"=".repeat(filled)),
        marker,
        theme::dim(&".".repeat(remaining.saturating_sub(1))),
    );
    // pad_cell adds the trailing gutter and clamps any overshoot to exactly width.
    pad_cell(&format!(" {}{bar}", theme::dim(&cont)), width)
}

/// The tree prefix for an agent's bar line: its own connector becomes a vertical
/// (`├ `→`│ `) or blank (`└ `→`  `) so the stalk flows down past the bar to the
/// agent's following siblings/children.
fn bar_continuation(prefix: &str) -> String {
    if let Some(head) = prefix.strip_suffix("├ ") {
        format!("{head}│ ")
    } else if let Some(head) = prefix.strip_suffix("└ ") {
        format!("{head}  ")
    } else {
        prefix.to_string()
    }
}

/// Colour a panel row's NAME by status (#820): green = running, orange/yellow =
/// idle, red = errored. Exited names dim out; unknown states stay uncoloured.
/// No glyph is emitted — the colour alone conveys the state.
pub(crate) fn status_colored_name(status: &str, name: &str) -> String {
    match status {
        "running" | "starting" => theme::green(name),
        "idle" => theme::yellow(name),
        // #1461 persisted-roster liveness: dead reads as an error state,
        // detached dims like exited so it can't pass for a live agent.
        "error" | "errored" | "dead" => theme::red(name),
        "exited" | "detached" => theme::dim(name),
        _ => name.to_string(),
    }
}

/// Strip terminal control sequences from a panel label.
pub(crate) fn sanitize_panel_label(s: &str) -> String {
    crate::components::ansi::sanitize_control(s)
}

/// Pad (or truncate) a cell to exactly `width` visible columns.
pub(crate) fn pad_cell(text: &str, width: usize) -> String {
    let visible = crate::components::utils::visible_width(text);
    if visible > width {
        crate::components::utils::truncate_to_width(text, width, None)
    } else {
        format!("{}{}", text, " ".repeat(width - visible))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{ansi::strip_ansi, utils::visible_width};

    #[test]
    fn ascii_workflow_bar_states_and_colours() {
        for (done, expected) in [(0, ">...."), (3, "===>."), (5, "====="), (9, "=====")] {
            let line = panel_bar_line("", done, 5, 7);
            assert_eq!(strip_ansi(&line), format!(" {expected} "));
            assert!(line.contains(&theme::green(&"=".repeat(done.min(5) as usize))));
            assert_eq!(line.contains(&theme::yellow(">")), done < 5);
            if done < 5 {
                assert!(line.contains(&theme::dim(&".".repeat((4 - done) as usize))));
            }
        }
    }

    #[test]
    fn ascii_workflow_bar_preserves_cap_and_proportional_scaling() {
        assert_eq!(
            strip_ansi(&panel_bar_line("", 25, 100, 22)),
            " =====>.............. "
        );
        assert_eq!(strip_ansi(&panel_bar_line("", 50, 100, 12)), " =====>.... ");
        assert_eq!(strip_ansi(&panel_bar_line("", 99, 100, 12)), " =========> ");
        assert_eq!(
            strip_ansi(&panel_bar_line("", 100, 100, 12)),
            " ========== "
        );
    }

    #[test]
    fn ascii_workflow_bar_preserves_tree_and_single_row() {
        for (prefix, continuation) in [("├ ", "│ "), ("└ ", "  "), ("│ ├ ", "│ │ ")] {
            let line = panel_bar_line(prefix, 1, 3, 12);
            assert_eq!(
                strip_ansi(&line),
                format!(" {}=>.", continuation) + &" ".repeat(8 - visible_width(continuation))
            );
            assert_eq!(line.lines().count(), 1);
            assert_eq!(visible_width(&line), 12);
        }
    }

    #[test]
    fn ascii_workflow_bar_handles_narrow_and_zero_total() {
        // Retain the existing one-cell fallback, with pad_cell clamping overshoot.
        assert_eq!(strip_ansi(&panel_bar_line("", 0, 0, 3)), " > ");
        assert_eq!(strip_ansi(&panel_bar_line("", 0, 5, 3)), " > ");
        assert_eq!(strip_ansi(&panel_bar_line("", 5, 5, 3)), " = ");
        for prefix in ["", "├ ", "│ └ "] {
            for width in 0..=24 {
                for (done, total) in [(0, 0), (0, 5), (2, 5), (5, 5), (9, 5)] {
                    let line = panel_bar_line(prefix, done, total, width);
                    assert_eq!(visible_width(&line), width);
                    assert!(!line.contains('\n'));
                }
            }
        }
    }
}

#[cfg(test)]
/// Count completed and incomplete cells in panel-only workflow bar rows.
pub(crate) fn panel_markers(frame: &str) -> (usize, usize) {
    let cells: String = frame
        .lines()
        .filter_map(|line| {
            let bar = line.trim_start_matches([' ', '│']);
            bar.starts_with(['=', '>']).then_some(bar.trim_end())
        })
        .collect();
    (
        cells.matches('=').count(),
        cells.matches(['>', '.']).count(),
    )
}
