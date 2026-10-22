//! The commit a free-text revision names, as the push rule reads it
//! (#2666): a swarm's evidence revision, or the revision its run completed
//! at. One authority for the project board's end of a run and for the
//! teardown that keeps a succeeded run's box.

/// The shortest run of hex digits read as a commit id at all: git's own
/// abbreviation. One of 7 to 11 is counted, but not checked.
pub const MIN_COMMIT_TOKEN: usize = 7;

/// The shortest prefix of a commit id the push rule checks: a shorter run
/// of hex digits may be a word or a digest.
pub const MIN_COMMIT_PREFIX: usize = 12;

/// The commit id `revision` names, in lower case: of its words (split at
/// whitespace, `@` and `/`, each word's surrounding punctuation trimmed:
/// backticks, quotes, brackets, a full stop, a comma), the one that is
/// wholly hex digits, at least [`MIN_COMMIT_TOKEN`] of them (`abc…`,
/// `` `abc…`. ``, `commit abc…`, `abc… (main)`, `abc… added fib`,
/// `owner/repo@abc…`); or, when several are, the one of them that has at
/// least [`MIN_COMMIT_PREFIX`] (`abc… on 20261021`). None when no word
/// is, or more than one long one is: no id is guessed.
pub fn commit_token(revision: &str) -> Option<String> {
    let hex: Vec<&str> = revision
        .split(|c: char| c.is_whitespace() || c == '@' || c == '/')
        .map(|word| word.trim_matches(|c: char| !c.is_ascii_alphanumeric()))
        .filter(|word| {
            word.len() >= MIN_COMMIT_TOKEN && word.bytes().all(|b| b.is_ascii_hexdigit())
        })
        .collect();
    match hex.as_slice() {
        [token] => Some(token.to_ascii_lowercase()),
        [] => None,
        [_, _, ..] => {
            let long: Vec<&str> = hex
                .iter()
                .copied()
                .filter(|word| word.len() >= MIN_COMMIT_PREFIX)
                .collect();
            match long.as_slice() {
                [token] => Some(token.to_ascii_lowercase()),
                [] | [_, _, ..] => None,
            }
        }
    }
}

/// Whether commit token `id` is checked by the push rule in a repository
/// whose full ids are `id_len` hex digits (40 SHA-1, 64 SHA-256): a full
/// id or a prefix of at least [`MIN_COMMIT_PREFIX`]. Longer than a full id
/// (a SHA-256 digest in a SHA-1 repository) or shorter is not.
pub fn checks_commit(id: &str, id_len: usize) -> bool {
    (MIN_COMMIT_PREFIX..=id_len).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_hexdigit())
}
