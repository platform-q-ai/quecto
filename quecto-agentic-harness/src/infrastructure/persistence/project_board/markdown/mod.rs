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
    let mut lines = vec![
        format!("<!-- quecto-{} schema={} -->", shape.kind, document.schema),
        format!("# {}", title(&document.title)?),
    ];
    if !document.fields.is_empty() {
        lines.extend(["", "| Field | Value |", "|---|---|"].map(String::from));
    }
    let mut order = shape.fields.iter();
    for (name, value) in &document.fields {
        match (
            shape.fields.contains(&name.as_str()),
            order.any(|field| field == name),
        ) {
            (true, true) => {}
            (false, _) => return Err(MarkdownError::UnknownField { name: name.clone() }),
            (true, false) => return Err(out_of_order(&lines, "fields in the kind's order")),
        }
        text::check_cell(value, lines.len() + 1)?;
        lines.push(format!("| {name} | {} |", text::escaped_cell(value)));
    }
    let mut order = shape.sections.iter();
    for section in &document.sections {
        let unknown = || MarkdownError::UnknownSection {
            heading: section.heading.clone(),
        };
        let known = shape
            .sections
            .iter()
            .find(|known| known.heading == section.heading);
        let known = known.ok_or_else(unknown)?;
        match order.any(|next| next.heading == section.heading) {
            true => lines.extend([String::new(), format!("## {}", section.heading)]),
            false => return Err(out_of_order(&lines, "sections in the kind's order")),
        }
        match (&section.body, known.body) {
            (Body::Prose(prose), BodyShape::Prose) => {
                push_prose(&mut lines, &section.heading, prose)?
            }
            (Body::Table(rows), BodyShape::Table(columns)) => {
                push_table(&mut lines, columns, rows)?
            }
            _ => return Err(unknown()),
        }
    }
    within(&lines, shape.limits)?;
    Ok(lines.join("\n") + "\n")
}

fn out_of_order(lines: &[String], expected: &'static str) -> MarkdownError {
    MarkdownError::Structure {
        line: lines.len() + 1,
        expected,
    }
}

/// A title: one line, nothing at its edges, and no closing `#` (which a
/// heading drops).
fn title(title: &str) -> Result<&str, MarkdownError> {
    text::check_text(title, 2)?;
    let edges = title.trim() != title || title.ends_with('#');
    match title.is_empty() || title.contains('\n') || edges {
        true => Err(MarkdownError::Title),
        false => Ok(title),
    }
}

/// Prose: paragraphs of lines, one blank line between them, each line that
/// starts like a block escaped.
fn push_prose(lines: &mut Vec<String>, heading: &str, prose: &str) -> Result<(), MarkdownError> {
    let paragraphs: Vec<&str> = prose.split('\n').collect();
    let blank_twice = paragraphs.windows(2).any(|pair| pair == ["", ""]);
    let blank_edge = paragraphs.first() == Some(&"") || paragraphs.last() == Some(&"");
    match (prose.is_empty(), blank_twice || blank_edge) {
        (true, _) => {
            return Err(MarkdownError::EmptyProse {
                heading: heading.into(),
            });
        }
        (false, true) => return Err(out_of_order(lines, "one blank line between paragraphs")),
        (false, false) => {}
    }
    for line in paragraphs {
        text::check_text(line, lines.len() + 1)?;
        let escape = if text::starts_like_block(line) {
            "\\"
        } else {
            ""
        };
        lines.push(format!("{escape}{line}"));
    }
    Ok(())
}

/// A table: its header, its separator, and each row as wide as the header.
fn push_table(
    lines: &mut Vec<String>,
    columns: &[&str],
    rows: &[Vec<String>],
) -> Result<(), MarkdownError> {
    lines.push(format!("| {} |", columns.join(" | ")));
    lines.push(format!("|{}", "---|".repeat(columns.len())));
    for row in rows {
        let at = lines.len() + 1;
        if row.len() != columns.len() {
            return Err(MarkdownError::Cell {
                line: at,
                why: "a row of another width",
            });
        }
        let cells: Vec<String> = row
            .iter()
            .map(|cell| text::check_cell(cell, at).map(|()| text::escaped_cell(cell)))
            .collect::<Result<_, _>>()?;
        lines.push(format!("| {} |", cells.join(" | ")));
    }
    Ok(())
}

/// The file's lines keep the character rules and the limits.
fn within(lines: &[String], limits: Limits) -> Result<(), MarkdownError> {
    for (index, line) in lines.iter().enumerate() {
        text::check_line(line, index + 1)?;
        if line.len() > limits.line_bytes {
            return Err(MarkdownError::LineTooLong {
                line: index + 1,
                most: limits.line_bytes,
            });
        }
    }
    let bytes = lines.iter().map(|line| line.len() + 1).sum::<usize>();
    match (lines.len() <= limits.lines, bytes <= limits.file_bytes) {
        (true, true) => Ok(()),
        (false, _) => Err(MarkdownError::TooManyLines {
            lines: lines.len(),
            most: limits.lines,
        }),
        (true, false) => Err(MarkdownError::TooLarge {
            bytes,
            most: limits.file_bytes,
        }),
    }
}

#[cfg(test)]
#[path = "markdown_tests.rs"]
mod tests;
