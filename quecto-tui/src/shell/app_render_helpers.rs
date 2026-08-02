use crate::components::theme;

/// Animated "N subagent(s) working…" line shown in the reserved spinner slot
/// while the parent is idle but children are still active.
pub(super) fn subagent_activity_line(active: usize, frame: usize) -> String {
    use crate::components::theme::SPINNER_FRAMES;
    let spin = theme::spinner(SPINNER_FRAMES[frame % SPINNER_FRAMES.len()]);
    let noun = if active == 1 { "subagent" } else { "subagents" };
    format!(
        "  {} {}",
        spin,
        theme::muted(&format!("{active} {noun} working..."))
    )
}

/// Visible tracked-child idle placeholder; preserves chat height without a blank row.
pub(super) fn subagent_idle_line(tracked: usize) -> String {
    let noun = if tracked == 1 {
        "subagent"
    } else {
        "subagents"
    };
    format!("    {}", theme::muted(&format!("{tracked} {noun} idle")))
}

/// Strip ANSI escape sequences (CSI + OSC) for the render-log diagnostic and
/// the headless test harness.
pub(super) fn strip_ansi(s: &str) -> String {
    crate::components::ansi::strip_ansi(s)
}
