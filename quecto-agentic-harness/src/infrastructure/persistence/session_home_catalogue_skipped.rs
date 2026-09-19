//! Naming what the store's walk skipped in a metadata answer (R2-H1), once
//! per file and in time linear in the number of bad records (R3-H1).
use std::collections::HashSet;

/// A record the store's walk skipped is named once per answer: by the
/// catalogue's own line when it rejected the record too, else by the walk's.
/// A line names `file` when it starts with `"{file}: "`, so every prefix of a
/// line that ends before a `": "` is a name already taken — collected once.
pub(in crate::infrastructure::persistence) fn name_skipped(
    diagnostics: &mut Vec<String>,
    skipped: Vec<(String, String)>,
) {
    let mut named: HashSet<String> = HashSet::new();
    for line in diagnostics.iter() {
        named.extend(
            line.match_indices(": ")
                .map(|(at, _)| line[..at].to_string()),
        );
    }
    for (file, why) in skipped {
        let line = format!("{file}: session record not listed: {why}");
        if !named.contains(&file) {
            named.extend(
                line.match_indices(": ")
                    .map(|(at, _)| line[..at].to_string()),
            );
            diagnostics.push(line);
        }
    }
}

#[cfg(test)]
#[path = "session_home_catalogue_skipped_tests.rs"]
mod tests;
