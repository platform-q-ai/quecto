//! Reading a file back into its document.
use super::{Document, MarkdownError, Shape};

/// The document `bytes` holds, only if rendering it gives `bytes` again.
///
/// # Errors
/// Why the file is refused.
pub fn parse(bytes: &[u8], shape: &Shape<'_>) -> Result<Document, MarkdownError> {
    let _ = (bytes, shape);
    Err(MarkdownError::NotCanonical)
}
