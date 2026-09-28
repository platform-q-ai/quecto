//! The member's final report (#2285).

/// A final report is delivered in pages of this many bytes, the same
/// budget as a quecto child's final report (#2114).
pub const FINAL_REPORT_PAGE_BYTES: usize = 64 * 1024;

/// The member's final report: the last turn's `result` text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalReport {
    pub content: String,
    /// The last assistant message with text when the report arrived.
    pub message_ordinal: Option<u64>,
}

impl FinalReport {
    pub fn full_length_bytes(&self) -> usize {
        self.content.len()
    }

    /// The report split into pages of at most [`FINAL_REPORT_PAGE_BYTES`],
    /// each cut on a character boundary. An empty report is one empty page.
    pub fn pages(&self) -> Vec<&str> {
        let mut pages = Vec::new();
        let mut rest = self.content.as_str();
        loop {
            let mut end = rest.len().min(FINAL_REPORT_PAGE_BYTES);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            // A page always advances unless the rest is empty.
            debug_assert!(end > 0 || rest.is_empty());
            let (page, tail) = rest.split_at(end);
            pages.push(page);
            rest = tail;
            if rest.is_empty() {
                return pages;
            }
        }
    }
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
