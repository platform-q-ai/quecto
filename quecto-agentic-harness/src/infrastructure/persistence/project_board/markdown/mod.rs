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
//! A paragraph, \# its block-like lines escaped.
//!
//! - a list item
//!
//! ## Rows
//! | Column | Column |
//! |---|---|
//! | cell | cell |
//! ```
//!
//! One final LF; no CR or BOM. Every line keeps a character allowlist
//! ([`chars`]) and every text the inline rules ([`inline`]); prose is
//! paragraphs, one-level lists and fenced code ([`prose`]). Hostile text is
//! refused when it is written, never escaped into something else. Files,
//! lines and line counts are capped by the caller's [`Limits`].
//!
//! The core is generic: each kind's fields and sections, in order, are the
//! caller's [`Shape`].
mod chars;
mod inline;
mod prose;

/// How large a file, how many lines and how long a line may be: the
/// caller's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub file_bytes: usize,
    pub lines: usize,
    pub line_bytes: usize,
}

/// What a cell holds: text, or an id or pointer of ASCII letters, digits
/// and `._:/#@+-` (a slug, a commit, a time, an address).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellKind {
    Text,
    Ascii,
}

/// A field or a table's column: its name and what its cells hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Column<'a> {
    pub name: &'a str,
    pub cell: CellKind,
}

/// What a section holds: prose, or a table of these columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyShape<'a> {
    Prose,
    Table(&'a [Column<'a>]),
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
    pub fields: &'a [Column<'a>],
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    Prose(Vec<Block>),
    /// Rows under the shape's columns.
    Table(Vec<Vec<String>>),
}

/// One block of prose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Lines of text, one paragraph.
    Paragraph(Vec<String>),
    /// A one-level `- ` list, an item a line.
    List(Vec<String>),
    /// Fenced code, its lines verbatim.
    Code {
        language: Option<String>,
        lines: Vec<String>,
    },
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
    /// A character outside the allowlist: an ignorable or invisible one
    /// (an emoji's joiner or selector aside), a separator, a private-use or
    /// unassigned one.
    Invisible {
        line: usize,
    },
    /// More combining marks on one base than [`chars::MOST_MARKS`].
    Marks {
        line: usize,
    },
    NotNfc {
        line: usize,
    },
    Html {
        line: usize,
    },
    CharacterReference {
        line: usize,
    },
    Math {
        line: usize,
    },
    Image {
        line: usize,
    },
    /// A link of another form than `[text](https://...)`, or a bare one.
    Link {
        line: usize,
    },
    Escape {
        line: usize,
    },
    UnbalancedCode {
        line: usize,
    },
    Fence {
        line: usize,
        why: &'static str,
    },
    /// A prose line that starts as a block would, unescaped.
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
    /// The caller's shape is not one a file can be.
    Shape(&'static str),
    /// Parsed, but the file is not the one rendering it gives.
    NotCanonical,
}

/// A name of a field, heading or column: an ASCII capital, then letters,
/// digits and single spaces, at most forty.
fn name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let allowed = |b: &u8| b.is_ascii_alphanumeric() || *b == b' ';
    (1..=40).contains(&bytes.len())
        && bytes[0].is_ascii_uppercase()
        && bytes.iter().all(allowed)
        && !name.contains("  ")
        && !name.ends_with(' ')
}

fn unique<'a>(names: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = std::collections::BTreeSet::new();
    names.into_iter().all(|name| seen.insert(name))
}

/// The caller's shape is one a file can be.
///
/// # Errors
/// What in the shape is not.
pub fn check_shape(shape: &Shape<'_>) -> Result<(), MarkdownError> {
    let kind =
        (1..=24).contains(&shape.kind.len()) && shape.kind.bytes().all(|b| b.is_ascii_lowercase());
    let fields = shape.fields.iter().all(|field| name(field.name))
        && unique(shape.fields.iter().map(|f| f.name));
    let headings = shape.sections.iter().map(|section| section.heading);
    let tables = shape.sections.iter().all(|section| match section.body {
        BodyShape::Prose => true,
        BodyShape::Table(columns) => {
            !columns.is_empty()
                && columns.iter().all(|c| name(c.name))
                && unique(columns.iter().map(|c| c.name))
        }
    });
    let sections = shape.sections.iter().all(|section| name(section.heading)) && unique(headings);
    match (kind, fields, sections, tables) {
        (true, true, true, true) => Ok(()),
        (false, ..) => Err(MarkdownError::Shape("a kind of 1 to 24 a-z")),
        (_, false, ..) => Err(MarkdownError::Shape("field names, each once")),
        (_, _, false, _) => Err(MarkdownError::Shape("headings, each once")),
        (.., false) => Err(MarkdownError::Shape("tables of named columns, each once")),
    }
}

/// The bytes of text a document holds, before any is rendered.
fn text_bytes(document: &Document) -> usize {
    let fields = document
        .fields
        .iter()
        .map(|(name, value)| name.len() + value.len());
    let sections = document.sections.iter().map(|section| {
        section.heading.len()
            + match &section.body {
                Body::Table(rows) => rows.iter().flatten().map(String::len).sum::<usize>(),
                Body::Prose(blocks) => blocks
                    .iter()
                    .map(|block| match block {
                        Block::Paragraph(texts) | Block::List(texts) => {
                            texts.iter().map(String::len).sum()
                        }
                        Block::Code { language, lines } => {
                            language.as_ref().map_or(0, String::len)
                                + lines.iter().map(String::len).sum::<usize>()
                        }
                    })
                    .sum(),
            }
    });
    document.title.len() + fields.sum::<usize>() + sections.sum::<usize>()
}

/// The lines `document`'s file has, counted before anything is rendered
/// (a text holding a newline is refused as it is rendered).
fn file_lines(document: &Document) -> usize {
    let fields = match document.fields.len() {
        0 => 0,
        rows => 3 + rows,
    };
    let sections = document.sections.iter().map(|section| {
        2 + match &section.body {
            Body::Table(rows) => 2 + rows.len(),
            Body::Prose(blocks) => {
                let between = blocks.len().saturating_sub(1);
                between
                    + blocks
                        .iter()
                        .map(|block| match block {
                            Block::Paragraph(texts) | Block::List(texts) => texts.len(),
                            Block::Code { lines, .. } => 2 + lines.len(),
                        })
                        .sum::<usize>()
            }
        }
    });
    2 + fields + sections.sum::<usize>()
}

/// A cell as a row holds it.
fn cell(value: &str, kind: CellKind, at: usize) -> Result<String, MarkdownError> {
    let refused = |why| Err(MarkdownError::Cell { line: at, why });
    let ascii = |b: u8| b.is_ascii_alphanumeric() || b"._:/#@+-".contains(&b);
    match (kind, value) {
        (_, "") => refused("an empty cell"),
        (_, value) if value.contains('\n') => refused("a newline in a cell"),
        (_, value) if value.starts_with(' ') || value.ends_with(' ') => {
            refused("space at a cell's edge")
        }
        (CellKind::Ascii, value) if value.bytes().all(ascii) => Ok(value.to_string()),
        (CellKind::Ascii, _) => refused("an id or pointer of ASCII letters, digits and ._:/#@+-"),
        (CellKind::Text, value) => {
            inline::check_inline(value, at).map(|()| value.replace('|', "\\|"))
        }
    }
}

/// The file `document` is, refused when it holds anything a file must not.
///
/// # Errors
/// What in the shape or document is refused, and where its file would
/// hold it.
pub fn render(document: &Document, shape: &Shape<'_>) -> Result<String, MarkdownError> {
    check_shape(shape)?;
    let limits = shape.limits;
    match text_bytes(document) <= limits.file_bytes {
        true => {}
        false => {
            return Err(MarkdownError::TooLarge {
                bytes: text_bytes(document),
                most: limits.file_bytes,
            });
        }
    }
    let counted = file_lines(document);
    if counted > limits.lines {
        return Err(MarkdownError::TooManyLines {
            lines: counted,
            most: limits.lines,
        });
    }
    let mut lines = vec![
        format!("<!-- quecto-{} schema={} -->", shape.kind, document.schema),
        format!("# {}", title(&document.title)?),
    ];
    if !document.fields.is_empty() {
        lines.extend(["", "| Field | Value |", "|---|---|"].map(String::from));
    }
    let mut order = shape.fields.iter();
    for (name, value) in &document.fields {
        let known = shape.fields.iter().find(|field| field.name == name);
        let known = known.ok_or_else(|| MarkdownError::UnknownField { name: name.clone() })?;
        match order.any(|field| field.name == name) {
            true => {}
            false => return Err(out_of_order(&lines, "fields in the kind's order")),
        }
        let value = cell(value, known.cell, lines.len() + 1)?;
        lines.push(format!("| {name} | {value} |"));
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
            (Body::Prose(blocks), BodyShape::Prose) => {
                prose::render(&mut lines, &section.heading, blocks)?
            }
            (Body::Table(rows), BodyShape::Table(columns)) => table(&mut lines, columns, rows)?,
            _ => return Err(unknown()),
        }
    }
    within(&lines, limits)?;
    debug_assert_eq!(
        lines.len(),
        counted,
        "the lines counted are the lines written"
    );
    Ok(lines.join("\n") + "\n")
}

fn out_of_order(lines: &[String], expected: &'static str) -> MarkdownError {
    MarkdownError::Structure {
        line: lines.len() + 1,
        expected,
    }
}

/// A title: one line of text with nothing at its edges and no closing `#`
/// (which a heading drops).
fn title(title: &str) -> Result<&str, MarkdownError> {
    let edges = title.trim() != title || title.ends_with('#');
    match title.is_empty() || title.contains('\n') || edges {
        true => Err(MarkdownError::Title),
        false => inline::check_inline(title, 2).map(|()| title),
    }
}

/// A table: its header, its separator, and each row as wide as the header.
fn table(
    lines: &mut Vec<String>,
    columns: &[Column<'_>],
    rows: &[Vec<String>],
) -> Result<(), MarkdownError> {
    let names: Vec<&str> = columns.iter().map(|column| column.name).collect();
    lines.push(format!("| {} |", names.join(" | ")));
    lines.push(format!("|{}", "---|".repeat(columns.len())));
    for row in rows {
        let at = lines.len() + 1;
        if row.len() != columns.len() {
            return Err(MarkdownError::Cell {
                line: at,
                why: "a row of another width",
            });
        }
        let cells = row
            .iter()
            .zip(columns)
            .map(|(value, column)| cell(value, column.cell, at))
            .collect::<Result<Vec<_>, _>>()?;
        lines.push(format!("| {} |", cells.join(" | ")));
    }
    Ok(())
}

/// The file's lines keep the character rules and the limits.
fn within(lines: &[String], limits: Limits) -> Result<(), MarkdownError> {
    for (index, line) in lines.iter().enumerate() {
        chars::check_line(line, index + 1)?;
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
