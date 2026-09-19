//! The freshness tail of a discovery answer — `list_sessions` and
//! `search_session_metadata` alike (#2010): `rebuilt`, and the diagnostics
//! made safe and BOUNDED (R2-H5). A store with a thousand corrupt records
//! would otherwise put a thousand lines into every answer, one per keystroke.

#[path = "uds_safe_display.rs"]
mod uds_safe_display;
pub(in crate::interface::cli) use uds_safe_display::safe_display;

/// The diagnostics an answer names in full; the rest are counted.
pub(in crate::interface::cli) const MAX_DIAGNOSTICS: usize = 20;

/// `body` with `diagnostics` (the first [`MAX_DIAGNOSTICS`] lines, then one
/// `… and N more` line when any were left out), `diagnosticsTotal` (every
/// line there was) and `rebuilt`.
pub(in crate::interface::cli) fn freshened(
    mut body: serde_json::Value,
    diagnostics: &[String],
    rebuilt: bool,
) -> serde_json::Value {
    let shown = diagnostics.iter().take(MAX_DIAGNOSTICS);
    let mut lines: Vec<String> = shown.map(|line| safe_display(line)).collect();
    if diagnostics.len() > MAX_DIAGNOSTICS {
        lines.push(format!(
            "… and {} more",
            diagnostics.len() - MAX_DIAGNOSTICS
        ));
    }
    body["diagnostics"] = serde_json::json!(lines);
    body["diagnosticsTotal"] = serde_json::json!(diagnostics.len());
    body["rebuilt"] = serde_json::json!(rebuilt);
    body
}
