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
mod tests {
    use super::*;

    #[test]
    fn each_diagnostic_toasts_once_per_process_and_batches_summarise() {
        let mut shown = BTreeSet::new();
        let a = "a.json: session record unavailable: expected value".to_string();
        let b = "b.json: session record unavailable: EOF".to_string();
        assert_eq!(
            unseen_diagnostics_toast(&mut shown, std::slice::from_ref(&a)),
            Some(a.clone())
        );
        // Same listing again (other scope, or a second /resume): silent.
        assert_eq!(
            unseen_diagnostics_toast(&mut shown, std::slice::from_ref(&a)),
            None
        );
        assert_eq!(unseen_diagnostics_toast(&mut shown, &[]), None);
        // Several new ones (duplicates within a batch count once): one summary.
        let c = "c.json: home needs repair: missing".to_string();
        assert_eq!(
            unseen_diagnostics_toast(&mut shown, &[a.clone(), b.clone(), b.clone(), c.clone()]),
            Some(format!("2 session records need repair; first: {b}"))
        );
        assert_eq!(unseen_diagnostics_toast(&mut shown, &[b, c]), None);
    }
}
