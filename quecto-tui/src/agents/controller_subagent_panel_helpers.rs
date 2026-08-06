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

/// Per-step workflow bar beneath an agent's name (`▰` done · `▱` pending), one
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
    let bar = format!(
        "{}{}",
        theme::accent(&"▰".repeat(filled)),
        theme::dim(&"▱".repeat(cells - filled)),
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
        "error" | "errored" => theme::red(name),
        "exited" => theme::dim(name),
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
