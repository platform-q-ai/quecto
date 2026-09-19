//! The search box against a harness that cannot search (R1-T5): a literal
//! filter over the rows the picker already listed, by the harness's own rule
//! — every word of the visible, case-folded text must occur in the title or
//! the folder, or the whole text must BE the key. No repository label (a
//! listing carries none), no ranking: the listing's order is kept.
use crate::protocol::session_payloads::ResumeSessionSummary;

/// The listed rows `query` names, in listing order.
pub fn filter_listed(listed: &[ResumeSessionSummary], query: &str) -> Vec<ResumeSessionSummary> {
    let terms = visible_text(query);
    let terms: Vec<&str> = terms.split(' ').filter(|term| !term.is_empty()).collect();
    let matches = |row: &&ResumeSessionSummary| {
        let texts = [
            Some(visible_text(&row.title)),
            row.execution_dir.as_deref().map(visible_text),
        ];
        let found = |term: &&str| texts.iter().flatten().any(|text| text.contains(*term));
        row.key == query.trim() || terms.iter().all(found)
    };
    listed.iter().filter(matches).cloned().collect()
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

fn invisible(ch: char) -> bool {
    (ch.is_control() && !ch.is_whitespace())
        || matches!(ch,
            '\u{ad}' | '\u{34f}' | '\u{61c}' | '\u{180b}'..='\u{180f}'
            | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}'
            | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{e0000}'..='\u{e007f}')
}

#[cfg(test)]
#[path = "local_filter_tests.rs"]
mod tests;
