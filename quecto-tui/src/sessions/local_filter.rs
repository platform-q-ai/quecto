//! The search box against a harness that cannot search (R1-T5): a literal
//! filter over the rows the picker already listed, by the harness's own rule
//! as far as a listing's rows allow — every word of the visible, case-folded
//! text must occur in the title or the folder, or the whole text must BE the
//! key. No repository label (a listing carries none), no ranking: the
//! listing's order is kept.
//!
//! The folder text follows the harness's scope rule (R2-T8). All Folders: the
//! whole path. Local Folder: only what lies BELOW the local group's root, so
//! the repository's own name does not match every local row. A listing names
//! no group root; the deepest folder common to every listed row stands in for
//! it. Two differences, both on the over-matching side. A linked worktree
//! outside the repository makes that common folder their shared parent, so
//! the repository's name matches its rows again (the harness would match the
//! worktree's only). And when every listed row is in ONE folder, that folder
//! cannot be told from the group root and nothing would lie below it: the
//! whole path is matched then (R3-T6). (Rows spread over sub-folders of one
//! sub-folder still stand in for the root: its own name matches none.)
use crate::protocol::session_payloads::{ResumeSessionSummary, SessionListScope};
use std::path::{Path, PathBuf};

/// The listed rows `query` names in `scope`, in listing order.
pub fn filter_listed(
    listed: &[ResumeSessionSummary],
    query: &str,
    scope: SessionListScope,
) -> Vec<ResumeSessionSummary> {
    let terms = visible_text(query);
    let terms: Vec<&str> = terms.split(' ').filter(|term| !term.is_empty()).collect();
    let root = match scope {
        SessionListScope::Local => common_folder(listed),
        SessionListScope::Global => None,
    };
    let matches = |row: &&ResumeSessionSummary| {
        let folder = row.execution_dir.as_deref().map(|dir| {
            let below = root
                .as_ref()
                .and_then(|root| Path::new(dir).strip_prefix(root).ok());
            below.map_or_else(
                || visible_text(dir),
                |below| visible_text(&below.to_string_lossy()),
            )
        });
        let texts = [Some(visible_text(&row.title)), folder];
        let found = |term: &&str| texts.iter().flatten().any(|text| text.contains(*term));
        row.key == query.trim() || terms.iter().all(found)
    };
    listed.iter().filter(matches).cloned().collect()
}

/// The deepest folder every listed row's path lies in or under — `None`
/// when no row lies BELOW it (all rows are in one folder, R3-T6).
fn common_folder(listed: &[ResumeSessionSummary]) -> Option<PathBuf> {
    let dirs = || listed.iter().filter_map(|row| row.execution_dir.as_deref());
    let mut common = PathBuf::from(dirs().next()?);
    for dir in dirs() {
        while !Path::new(dir).starts_with(&common) {
            common.pop();
        }
    }
    let spread = dirs().any(|dir| Path::new(dir) != common);
    spread.then_some(common)
}

/// The harness's visible-text fold (`domain/session_metadata_text.rs`), kept
/// in step by hand — the two crates share no code: invisible and control
/// characters dropped, case folded (final sigma, sharp s, dotted/dotless i),
/// whitespace collapsed.
fn visible_text(raw: &str) -> String {
    let shown: String = raw.chars().filter(|ch| !invisible(*ch)).collect();
    let mut folded = String::with_capacity(shown.len());
    for ch in shown.to_lowercase().chars() {
        match ch {
            'ς' => folded.push('σ'),
            'ß' => folded.push_str("ss"),
            'ı' => folded.push('i'),
            '\u{307}' if folded.ends_with('i') => {}
            other => folded.push(other),
        }
    }
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What the harness's fold drops; the search box accepts none of it either.
pub(super) fn invisible(ch: char) -> bool {
    (ch.is_control() && !ch.is_whitespace())
        || matches!(ch,
            '\u{ad}' | '\u{34f}' | '\u{61c}' | '\u{180b}'..='\u{180f}'
            | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}'
            | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{e0000}'..='\u{e007f}')
}

#[cfg(test)]
#[path = "local_filter_tests.rs"]
mod tests;
