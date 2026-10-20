//! What failed in a JSON document's text (#2617): the one wording every
//! reader of a configuration, credential or registry file gives a document
//! that is not JSON. A value object: each side that parses with serde maps
//! its error's category to it, so the words have one owner.

/// A failure of a document's text itself, never of what it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentTextFailure {
    /// The text stops before the document does.
    EndsEarly,
    /// The text is not JSON.
    Malformed,
}

impl DocumentTextFailure {
    /// The failure in fixed words.
    pub fn words(self) -> &'static str {
        match self {
            Self::EndsEarly => "the file ends early",
            Self::Malformed => "malformed JSON",
        }
    }
}
