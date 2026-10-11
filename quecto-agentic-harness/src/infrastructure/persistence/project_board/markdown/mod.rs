//! The canonical markdown every board artifact is stored as (epic #2481,
//! M2a). A file is valid only if rendering what parsing it gives yields the
//! same bytes; anything else is set aside.
//!
//! ```text
//! <!-- quecto-<kind> schema=N -->
//! # Title
//!
//! | Field | Value |
//! |---|---|
//! | Name | value |
//!
//! ## Section
//! Prose, each line that starts like a block escaped with `\`.
//!
//! ## Rows
//! | Column | Column |
//! |---|---|
//! | cell | cell |
//! ```
//!
//! One final LF; no CR, no BOM, no trailing spaces. Hostile text is
//! refused when it is written, never escaped into something else: raw HTML
//! and comments, control, bidirectional and invisible characters (a
//! zero-width joiner only between emoji), unbalanced code spans, newlines
//! in cells. Files, lines and line counts are capped, files before they
//! are parsed. The parser takes exactly what the renderer emits.
//!
//! The core is generic: each kind's fields and sections, in order, are the
//! caller's [`Shape`].
mod text;

use super::task_record::MAX_TASK_FILE_BYTES;

/// How large a file, how many lines and how long a line may be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub file_bytes: usize,
    pub lines: usize,
    pub line_bytes: usize,
}

pub const LIMITS: Limits = Limits {
    file_bytes: MAX_TASK_FILE_BYTES,
    lines: 8_192,
    line_bytes: 4_096,
};

/// What a section holds: prose, or a table of these columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyShape<'a> {
    Prose,
    Table(&'a [&'a str]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionShape<'a> {
    pub heading: &'a str,
    pub body: BodyShape<'a>,
}

/// One kind's file: its marker's name, its fields and its sections, each
/// in the order a file lists them, any of them left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape<'a> {
    pub kind: &'a str,
    pub fields: &'a [&'a str],
    pub sections: &'a [SectionShape<'a>],
    pub limits: Limits,
}

/// One artifact file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub schema: u32,
    pub title: String,
    /// Field name and value, in the shape's order.
    pub fields: Vec<(String, String)>,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub heading: String,
    pub body: Body,
}

/// Prose (lines joined by `\n`; one blank line between paragraphs), or a
/// table's rows under the shape's header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Prose(String),
    Table(Vec<Vec<String>>),
}

/// Why a document or file was refused. `line` is the file's line, from 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkdownError {
    TooLarge {
        bytes: usize,
        most: usize,
    },
    TooManyLines {
        lines: usize,
        most: usize,
    },
    LineTooLong {
        line: usize,
        most: usize,
    },
    NotUtf8,
    ByteOrderMark,
    CarriageReturn {
        line: usize,
    },
    /// Not exactly one LF at the end.
    FinalNewline,
    TrailingSpace {
        line: usize,
    },
    Control {
        line: usize,
    },
    Bidi {
        line: usize,
    },
    /// A format, private-use or unassigned character, or a zero-width
    /// joiner outside an emoji sequence.
    Invisible {
        line: usize,
    },
    Html {
        line: usize,
    },
    UnbalancedCode {
        line: usize,
    },
    /// A prose line that starts like a block, unescaped.
    BlockLike {
        line: usize,
    },
    Marker,
    Title,
    /// Not what the grammar has at this line.
    Structure {
        line: usize,
        expected: &'static str,
    },
    UnknownField {
        name: String,
    },
    UnknownSection {
        heading: String,
    },
    Cell {
        line: usize,
        why: &'static str,
    },
    EmptyProse {
        heading: String,
    },
    /// Parsed, but the file is not the one rendering it gives.
    NotCanonical,
}

/// The file `document` is, refused when it holds anything a file must not.
///
/// # Errors
/// What in the document is refused, and where its file would hold it.
pub fn render(document: &Document, shape: &Shape<'_>) -> Result<String, MarkdownError> {
    let _ = (document, shape);
    Err(MarkdownError::NotCanonical)
}

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;
