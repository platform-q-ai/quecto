//! Discovery diagnostics toast policy (#2018): a diagnostic string is shown
//! once per TUI process, and several unseen diagnostics collapse into one
//! summary line rather than one toast each. The shown-set lives in the
//! sessions feature state; the shell only decides how to display the line.
use std::collections::BTreeSet;

/// Record `diagnostics` as shown and return the single line to toast, if any:
/// the diagnostic itself when exactly one is new, or
/// `N session records need repair; first: <diag>` when several are.
pub fn unseen_diagnostics_toast(
    shown: &mut BTreeSet<String>,
    diagnostics: &[String],
) -> Option<String> {
    let mut unseen: Vec<&String> = Vec::new();
    for diagnostic in diagnostics {
        if shown.insert(diagnostic.clone()) && !unseen.contains(&diagnostic) {
            unseen.push(diagnostic);
        }
    }
    match unseen.as_slice() {
        [] => None,
        [only] => Some((*only).clone()),
        [first, ..] => Some(format!(
            "{} session records need repair; first: {first}",
            unseen.len()
        )),
    }
}

#[cfg(test)]
#[path = "discovery_diagnostics_tests.rs"]
mod tests;
