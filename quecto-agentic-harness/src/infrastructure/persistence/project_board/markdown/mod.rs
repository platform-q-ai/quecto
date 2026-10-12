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
//! [`parse`] takes a file only if writing what it reads gives the same
//! bytes, so what the reader lets through the writer's rules still refuse.
//!
//! One final LF; no CR or BOM. Every line keeps a character allowlist
//! ([`chars`]) and every text the inline rules ([`inline`]); prose is
//! paragraphs, one-level lists and fenced code ([`prose`]). Hostile text is
//! refused when it is written, never escaped into something else. Files,
//! lines and line counts are capped by the caller's [`Limits`].
//!
//! Accepted, as visible text a reader may misread but which hides nothing:
//! - GitHub draws an emoji shortcode (`:smile:`) as its emoji; refusing it
//!   would refuse times such as `12:00:00`.
//! - A link may show a bare domain other than its address
//!   (`[paypal.com](https://evil.example)`): telling one apart would refuse
//!   ordinary text such as `README.md`, and board files are tool-written.
//!   A link never shows a scheme, `www.` or an email.
//! - Any script's letters are allowed, so mixed-script confusables,
//!   fullwidth forms and right-to-left letters (never bidi controls) may
//!   look like other text.
//!
//! The core is generic: each kind's fields and sections, in order, are the
//! caller's [`Shape`].
mod chars;
mod inline;
mod parse;
mod prose;

pub use parse::parse;

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
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MarkdownError {
    #[error("{bytes} bytes, past the {most} a file may hold")]
    TooLarge { bytes: usize, most: usize },
    #[error("{lines} lines, past the {most} a file may hold")]
    TooManyLines { lines: usize, most: usize },
    #[error("line {line}: longer than {most} bytes")]
    LineTooLong { line: usize, most: usize },
    #[error("not UTF-8")]
    NotUtf8,
    #[error("a byte order mark")]
    ByteOrderMark,
    #[error("line {line}: a carriage return")]
    CarriageReturn { line: usize },
    /// Not exactly one LF at the end.
    #[error("not exactly one newline at the end")]
    FinalNewline,
    #[error("line {line}: a space at the end")]
    TrailingSpace { line: usize },
    #[error("line {line}: a control character")]
    Control { line: usize },
    #[error("line {line}: a bidirectional control")]
    Bidi { line: usize },
    /// A character outside the allowlist: an ignorable or invisible one
    /// (an emoji's joiner or selector aside), a separator, a private-use or
    /// unassigned one.
    #[error("line {line}: a character a reader does not see, or nothing a reader sees")]
    Invisible { line: usize },
    /// More combining marks on one base than [`chars::MOST_MARKS`].
    #[error(
        "line {line}: more than {} combining marks on one character",
        chars::MOST_MARKS
    )]
    Marks { line: usize },
    #[error("line {line}: not in Unicode NFC")]
    NotNfc { line: usize },
    #[error(
        "line {line}: raw HTML, a comment or an autolink (`<` stands only before a space or at the end)"
    )]
    Html { line: usize },
    #[error("line {line}: a character reference")]
    CharacterReference { line: usize },
    #[error("line {line}: dollar math (a `$` touching anything but a space)")]
    Math { line: usize },
    #[error("line {line}: an image")]
    Image { line: usize },
    /// A link of another form than `[text](https://...)`, or a bare one.
    #[error("line {line}: a link of another form than [text](https://...), or a bare address")]
    Link { line: usize },
    #[error("line {line}: a backslash escape")]
    Escape { line: usize },
    #[error("line {line}: a code span left open")]
    UnbalancedCode { line: usize },
    #[error("line {line}: a code fence: {why}")]
    Fence { line: usize, why: &'static str },
    /// A prose line that starts as a block would, unescaped.
    #[error("line {line}: a line that starts as a block, unescaped")]
    BlockLike { line: usize },
    #[error("not this kind's marker line")]
    Marker,
    #[error("not a title: one line of text, no space at its edges, no closing hashes")]
    Title,
    /// Not what the grammar has at this line.
    #[error("line {line}: expected {expected}")]
    Structure { line: usize, expected: &'static str },
    #[error("a field this kind does not have: {name:?}")]
    UnknownField { name: String },
    #[error("a section this kind does not have: {heading:?}")]
    UnknownSection { heading: String },
    #[error("line {line}: a cell: {why}")]
    Cell { line: usize, why: &'static str },
    #[error("section {heading:?} holds no prose")]
    EmptyProse { heading: String },
    /// The caller's shape is not one a file can be.
    #[error("a shape no file can be: {0}")]
    Shape(&'static str),
    /// Parsed, but the file is not the one rendering it gives. Only the
    /// read side (`parse`, #2503) answers it; it is here so the codec has
    /// one error type for both sides.
    #[error("not the canonical file for what it holds")]
    NotCanonical,
    /// A file render wrote that does not read back as its document: why
    /// it was refused when read, or none when it read as another document.
    #[error("the written file does not read back as its document{}", why.as_ref().map(|why| format!(": {why}")).unwrap_or_default())]
    NotReadBack { why: Option<Box<MarkdownError>> },
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

/// Where `document`'s texts land, worked out before anything is
/// rendered: the lines its file has, and the first line whose texts alone
/// are longer than `most` bytes (a text holding a newline is refused as it
/// is rendered).
struct Layout {
    lines: usize,
    most: usize,
    long: Option<usize>,
}

impl Layout {
    fn of(document: &Document, most: usize) -> Self {
        // The marker's line.
        let mut layout = Self {
            lines: 1,
            most,
            long: None,
        };
        layout.line([&document.title]);
        if !document.fields.is_empty() {
            layout.skip(3);
            for (name, value) in &document.fields {
                layout.line([name, value]);
            }
        }
        for section in &document.sections {
            layout.skip(1);
            layout.line([&section.heading]);
            match &section.body {
                Body::Table(rows) => {
                    layout.skip(2);
                    for row in rows {
                        layout.line(row);
                    }
                }
                Body::Prose(blocks) => {
                    for (index, block) in blocks.iter().enumerate() {
                        layout.skip(usize::from(index > 0));
                        match block {
                            Block::Paragraph(texts) | Block::List(texts) => {
                                texts.iter().for_each(|text| layout.line([text]));
                            }
                            Block::Code { lines, .. } => {
                                layout.skip(1);
                                lines.iter().for_each(|line| layout.line([line]));
                                layout.skip(1);
                            }
                        }
                    }
                }
            }
        }
        layout
    }

    fn skip(&mut self, lines: usize) {
        self.lines += lines;
    }

    fn line<'a>(&mut self, texts: impl IntoIterator<Item = &'a String>) {
        self.lines += 1;
        let bytes: usize = texts.into_iter().map(String::len).sum();
        if self.long.is_none() && bytes > self.most {
            self.long = Some(self.lines);
        }
    }
}

/// What an ASCII cell is made of (an allowlist): an optional leading `@`
/// (`@scope/pkg`), then letters and digits, `.:/#+-`, and `_` only between
/// letters or digits, where no reader takes it for emphasis.
static ASCII_CELL: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"^@?(?:[A-Za-z0-9]+(?:_[A-Za-z0-9]+)*|[.:/#+-])+$").expect("a valid pattern")
});

/// Whether an ASCII cell shows as written, nothing in it linked or
/// emphasised: made as [`ASCII_CELL`] allows, and holding neither `www.`
/// nor `://`, the two forms GitHub would still link there (naming a form
/// is inherent to finding it; the oracle's comrak reader catches what a
/// pattern misses).
fn plain(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    ASCII_CELL.is_match(value) && !lower.contains("www.") && !lower.contains("://")
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
        (CellKind::Ascii, value) if value.bytes().all(ascii) && plain(value) => {
            Ok(value.to_string())
        }
        (CellKind::Ascii, value) if value.bytes().all(ascii) => {
            refused("an address a reader would link, or an underscore it would read as emphasis")
        }
        (CellKind::Ascii, _) => refused("an id or pointer of ASCII letters, digits and ._:/#@+-"),
        (CellKind::Text, value) => {
            inline::check_inline(value, at).map(|()| value.replace('|', "\\|"))
        }
    }
}

/// The file `document` is, refused when it holds anything a file must not.
/// What is written reads back as `document`, or is refused.
///
/// # Errors
/// What in the shape or document is refused, and where its file would
/// hold it; `NotReadBack` for a file that would not read back as it.
pub fn render(document: &Document, shape: &Shape<'_>) -> Result<String, MarkdownError> {
    // A real check, release builds included: a file that would not read
    // back as its document is never handed on to be stored.
    read_back(write(document, shape)?, document, shape)
}

/// `file`, written from `document`, if it reads back as `document`.
///
/// # Errors
/// `NotReadBack`, with why it was refused when read, when it does not.
fn read_back(
    file: String,
    document: &Document,
    shape: &Shape<'_>,
) -> Result<String, MarkdownError> {
    match parse(file.as_bytes(), shape) {
        Ok(read) if read == *document => Ok(file),
        Ok(_) => Err(MarkdownError::NotReadBack { why: None }),
        Err(why) => Err(MarkdownError::NotReadBack {
            why: Some(Box::new(why)),
        }),
    }
}

/// [`render`] without the read back, which [`parse`] compares against.
fn write(document: &Document, shape: &Shape<'_>) -> Result<String, MarkdownError> {
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
    let layout = Layout::of(document, limits.line_bytes);
    if layout.lines > limits.lines {
        return Err(MarkdownError::TooManyLines {
            lines: layout.lines,
            most: limits.lines,
        });
    }
    if let Some(line) = layout.long {
        return Err(MarkdownError::LineTooLong {
            line,
            most: limits.line_bytes,
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
        layout.lines,
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

/// A title: one line of text with nothing at its edges and no closing run
/// of `#` (hashes alone, or after a space), which a heading drops; `C#`
/// keeps its own.
fn title(title: &str) -> Result<&str, MarkdownError> {
    let edges = title.trim() != title;
    // A closing run a heading drops: hashes alone, or after a space.
    let open = title.trim_end_matches('#');
    let closed = open.len() < title.len() && (open.is_empty() || open.ends_with(' '));
    match title.is_empty() || title.contains('\n') || edges || closed {
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
