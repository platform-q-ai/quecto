//! What failed in a JSON document's text (#2617): the one wording every
//! reader of a configuration, credential or registry file gives a document
//! that is not JSON. The configuration readers
//! (`services::document_parse`) and the infrastructure's parse helper
//! (`infrastructure::persistence::file::json_failure`) both word a text
//! failure through it, so the words have one owner.

/// A failure of a JSON text itself, never of what it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentTextFailure {
    /// The text stops before the document does.
    EndsEarly,
    /// The text is not JSON.
    Malformed,
}

impl DocumentTextFailure {
    /// The failure `error` is. Text in memory is never an I/O failure, and
    /// a text failure is never one of data, so all but an early end is
    /// malformed text.
    pub fn of(error: &serde_json::Error) -> Self {
        use serde_json::error::Category;
        match error.classify() {
            Category::Eof => Self::EndsEarly,
            Category::Syntax | Category::Io | Category::Data => Self::Malformed,
        }
    }

    /// The failure in fixed words.
    pub fn words(self) -> &'static str {
        match self {
            Self::EndsEarly => "the file ends early",
            Self::Malformed => "malformed JSON",
        }
    }
}
