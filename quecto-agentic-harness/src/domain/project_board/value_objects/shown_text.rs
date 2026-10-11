//! How the board shows text from outside it (a name in a tree, a path
//! set aside): one allowlist, shared by every layer that shows such text
//! or checks it was shown so.
use regex::Regex;
use std::sync::LazyLock;

/// The characters a name from outside is shown with as they are: letters,
/// marks, numbers, punctuation and symbols. Anything else (spaces,
/// controls, format characters such as bidirectional overrides,
/// zero-width and tag characters, unassigned code points) is not.
static SHOWN_CHAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\p{L}\p{M}\p{N}\p{P}\p{S}]$").expect("a valid class"));

/// Whether `c` is shown as it is ([`SHOWN_CHAR`]).
pub fn shown_char(c: char) -> bool {
    SHOWN_CHAR.is_match(c.encode_utf8(&mut [0; 4]))
}

/// Whether `path` is a set-aside file's path as a store shows it: under
/// `tasks/`, every character shown as it is, the one space a store puts
/// for a run of any other, or the `…` that marks a name it cut.
pub fn shown_path(path: &str) -> bool {
    path.strip_prefix("tasks/")
        .is_some_and(|name| name.chars().all(|c| shown_char(c) || c == ' ' || c == '…'))
}

#[cfg(test)]
#[path = "shown_text_tests.rs"]
mod tests;
