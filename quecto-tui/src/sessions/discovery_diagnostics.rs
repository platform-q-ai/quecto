//! Discovery diagnostics toast policy (#2018): a diagnostic string is shown
//! once per TUI process, and several unseen diagnostics collapse into one
//! summary line naming each source rather than one toast each. The
//! shown-set lives in the sessions feature state; the shell only decides how
//! to display the line.
use std::collections::BTreeSet;

/// Sources (the text before the first `: `) listed in a batch summary.
const SUMMARY_SOURCES: usize = 3;

/// Record `diagnostics` as shown and return the single line to toast, if any:
/// the diagnostic itself when exactly one is new, or
/// `N session discovery problems: a.json, b.json, c.json, …; first: <diag>`
/// when several are — every source is named so the batch can be repaired.
pub fn unseen_diagnostics_toast(
    shown: &mut BTreeSet<String>,
    diagnostics: &[String],
) -> Option<String> {
    let unseen: Vec<&String> = diagnostics
        .iter()
        .filter(|diagnostic| shown.insert((*diagnostic).clone()))
        .collect();
    match unseen.as_slice() {
        [] => None,
        [only] => Some((*only).clone()),
        [first, ..] => {
            let mut sources: Vec<&str> = unseen
                .iter()
                .map(|d| d.split_once(": ").map_or(d.as_str(), |(source, _)| source))
                .collect();
            sources.dedup();
            let more = sources.len().saturating_sub(SUMMARY_SOURCES);
            let mut named = sources[..sources.len().min(SUMMARY_SOURCES)].join(", ");
            if more > 0 {
                named.push_str(&format!(", +{more} more"));
            }
            Some(format!(
                "{} session discovery problems: {named}; first: {first}",
                unseen.len()
            ))
        }
    }
}

#[cfg(test)]
#[path = "discovery_diagnostics_tests.rs"]
mod tests;
