//! Reading a file back into its document. The bytes are checked before
//! anything is read (the caller's limits, then every line's characters),
//! the grammar is read line by line, and the document is taken only if
//! writing it gives the same bytes: what the reader lets through, the
//! writer's rules still refuse.
use super::chars::check_line;
use super::prose::{LANGUAGE_RULE, defines_reference, language, unescaped};
use super::{Block, Body, BodyShape, Column, Document, MarkdownError, Section, Shape};
use super::{cell_value, check_shape, write};

/// The longest marker line: `<!-- quecto-`, a kind of at most 24, then
/// ` schema=`, at most ten digits and ` -->`.
const MOST_MARKER_BYTES: usize = 12 + 24 + 8 + 10 + 4;

/// The document `bytes` holds, only if writing it gives `bytes` again.
///
/// # Errors
/// Why the file is refused, and the line, from 1, where it can be named.
pub fn parse(bytes: &[u8], shape: &Shape<'_>) -> Result<Document, MarkdownError> {
    check_shape(shape)?;
    let limits = shape.limits;
    if bytes.len() > limits.file_bytes {
        return Err(MarkdownError::TooLarge {
            bytes: bytes.len(),
            most: limits.file_bytes,
        });
    }
    if bytes.starts_with("\u{feff}".as_bytes()) {
        return Err(MarkdownError::ByteOrderMark);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| MarkdownError::NotUtf8)?;
    let Some(body) = text.strip_suffix('\n').filter(|body| !body.ends_with('\n')) else {
        return Err(MarkdownError::FinalNewline);
    };
    // Counted before any line is collected.
    let count = body.bytes().filter(|byte| *byte == b'\n').count() + 1;
    if count > limits.lines {
        return Err(MarkdownError::TooManyLines {
            lines: count,
            most: limits.lines,
        });
    }
    let lines: Vec<&str> = body.split('\n').collect();
    debug_assert_eq!(lines.len(), count, "the lines counted are the lines read");
    for (index, line) in lines.iter().enumerate() {
        if line.len() > limits.line_bytes {
            return Err(MarkdownError::LineTooLong {
                line: index + 1,
                most: limits.line_bytes,
            });
        }
        check_line(line, index + 1)?;
    }
    let document = Reader {
        lines: &lines,
        at: 0,
    }
    .document(shape)?;
    match write(&document, shape)?.as_bytes() == bytes {
        true => Ok(document),
        false => Err(MarkdownError::NotCanonical),
    }
}

/// The schema the marker of `kind` on the first line of `bytes` names,
/// read from that line alone (whatever follows it), so a file a newer
/// schema wrote is told apart before anything else in it is read.
/// Up to ten digits: one past what a `u32` holds still reads, as a schema
/// newer than any.
pub fn schema_of(bytes: &[u8], kind: &str) -> Option<u64> {
    let end = bytes
        .iter()
        .take(MOST_MARKER_BYTES + 1)
        .position(|b| *b == b'\n')?;
    marker_number(std::str::from_utf8(&bytes[..end]).ok()?, kind)
}

/// The schema a marker line of `kind` names, when a document holds it.
fn marker(line: &str, kind: &str) -> Option<u32> {
    marker_number(line, kind).and_then(|schema| u32::try_from(schema).ok())
}

/// The number a marker line of `kind` names, written without a leading
/// zero, in at most ten digits.
fn marker_number(line: &str, kind: &str) -> Option<u64> {
    let digits = line
        .strip_prefix("<!-- quecto-")?
        .strip_prefix(kind)?
        .strip_prefix(" schema=")?
        .strip_suffix(" -->")?;
    let canonical = (1..=10).contains(&digits.len())
        && digits.bytes().all(|b| b.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    canonical.then(|| digits.parse().ok()).flatten()
}

/// The cells of a row `| a | b |`: each `\|` read as `|`, every other
/// backslash as itself, and a `|` anywhere but between ` ` and ` ` refused.
fn cells(row: &str, at: usize) -> Result<Vec<String>, MarkdownError> {
    let cell = |why| MarkdownError::Cell { line: at, why };
    let inner = row
        .strip_prefix("| ")
        .and_then(|row| row.strip_suffix(" |"))
        .ok_or_else(|| cell("a row: | cell | cell |"))?;
    let (mut cells, mut current) = (vec![], String::new());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\\', Some('|')) => {
                current.push('|');
                chars.next();
            }
            ('|', _) => match (current.strip_suffix(' '), chars.next()) {
                (Some(done), Some(' ')) => {
                    cells.push(done.to_string());
                    current.clear();
                }
                _ => return Err(cell("a cell edge without a space each side")),
            },
            (c, _) => current.push(c),
        }
    }
    cells.push(current);
    Ok(cells)
}

struct Reader<'a> {
    lines: &'a [&'a str],
    /// The next line, from 0.
    at: usize,
}

impl<'a> Reader<'a> {
    fn peek(&self, ahead: usize) -> Option<&'a str> {
        self.lines.get(self.at + ahead).copied()
    }

    /// The next line's number, from 1.
    fn line(&self) -> usize {
        self.at + 1
    }

    fn structure(&self, expected: &'static str) -> MarkdownError {
        MarkdownError::Structure {
            line: self.line(),
            expected,
        }
    }

    /// The next line, which must be `wanted`.
    fn expect(&mut self, wanted: &str, expected: &'static str) -> Result<(), MarkdownError> {
        match self.peek(0) == Some(wanted) {
            true => {
                self.at += 1;
                Ok(())
            }
            false => Err(self.structure(expected)),
        }
    }

    fn document(mut self, shape: &Shape<'_>) -> Result<Document, MarkdownError> {
        let schema = self.peek(0).and_then(|line| marker(line, shape.kind));
        let schema = schema.ok_or(MarkdownError::Marker)?;
        let title = self.peek(1).and_then(|line| line.strip_prefix("# "));
        let title = title.ok_or(MarkdownError::Title)?;
        self.at = 2;
        let mut fields = vec![];
        if let (Some(""), Some("| Field | Value |")) = (self.peek(0), self.peek(1)) {
            self.at += 2;
            self.expect("|---|---|", "the field table's separator")?;
            let first = self.line();
            for (index, row) in self.rows(2)?.into_iter().enumerate() {
                debug_assert_eq!(row.len(), 2, "rows(2) reads rows of two cells");
                // A row of another width is refused by `rows`; were one
                // let through, the file would not be the one written.
                let mut cells = row.into_iter();
                if let (Some(name), Some(value), None) = (cells.next(), cells.next(), cells.next())
                {
                    // An unknown field's cell is kept as read: writing
                    // refuses the field.
                    let value = match shape.fields.iter().find(|field| field.name == name) {
                        Some(field) => cell_value(value, field.cell, first + index)?,
                        None => value,
                    };
                    fields.push((name, value));
                }
            }
        }
        let mut sections = vec![];
        while self.peek(0).is_some() {
            self.expect("", "a blank line before a section")?;
            let heading = self.peek(0).and_then(|line| line.strip_prefix("## "));
            let heading = heading.ok_or_else(|| self.structure("## heading"))?;
            self.at += 1;
            let known = shape.sections.iter().find(|known| known.heading == heading);
            let known = known.ok_or_else(|| MarkdownError::UnknownSection {
                heading: heading.into(),
            })?;
            let body = match known.body {
                BodyShape::Prose => Body::Prose(self.prose(heading)?),
                BodyShape::Table(columns) => Body::Table(self.table(columns)?),
            };
            sections.push(Section {
                heading: heading.into(),
                body,
            });
        }
        Ok(Document {
            schema,
            title: title.into(),
            fields,
            sections,
        })
    }

    /// Rows of `width` cells, up to a blank line or the end.
    fn rows(&mut self, width: usize) -> Result<Vec<Vec<String>>, MarkdownError> {
        let mut rows = vec![];
        while let Some(line) = self.peek(0).filter(|line| !line.is_empty()) {
            let row = match line.starts_with('|') {
                true => cells(line, self.line())?,
                false => return Err(self.structure("a table row or a blank line")),
            };
            if row.len() != width {
                return Err(MarkdownError::Cell {
                    line: self.line(),
                    why: "a row of another width",
                });
            }
            rows.push(row);
            self.at += 1;
        }
        Ok(rows)
    }

    /// A table: the header of `columns`, its separator, then its rows.
    fn table(&mut self, columns: &[Column<'_>]) -> Result<Vec<Vec<String>>, MarkdownError> {
        let names: Vec<&str> = columns.iter().map(|column| column.name).collect();
        let header = format!("| {} |", names.join(" | "));
        self.expect(&header, "the table's header")?;
        let separator = format!("|{}", "---|".repeat(columns.len()));
        self.expect(&separator, "the table's separator")?;
        let first = self.line();
        let rows = self.rows(columns.len())?.into_iter().enumerate();
        rows.map(|(index, row)| {
            debug_assert_eq!(row.len(), columns.len(), "rows reads rows as wide as asked");
            row.into_iter()
                .zip(columns)
                .map(|(read, column)| cell_value(read, column.cell, first + index))
                .collect()
        })
        .collect()
    }

    /// Prose: blocks one blank line apart, up to the blank line before the
    /// next section or the end.
    fn prose(&mut self, heading: &str) -> Result<Vec<Block>, MarkdownError> {
        let mut blocks = vec![];
        loop {
            match self.peek(0) {
                Some(first) if !first.is_empty() => blocks.push(self.block(first)?),
                _ if blocks.is_empty() => {
                    return Err(MarkdownError::EmptyProse {
                        heading: heading.into(),
                    });
                }
                _ => return Err(self.structure("a block after one blank line")),
            }
            match (self.peek(0), self.peek(1)) {
                (None, _) => return Ok(blocks),
                // A paragraph's `#` is escaped: only a section starts so.
                (Some(""), Some(next)) if next.starts_with("## ") => return Ok(blocks),
                (Some(""), Some(_)) => self.at += 1,
                _ => return Err(self.structure("a blank line between blocks")),
            }
        }
    }

    /// The block `first` opens: a fence (whose info a reader takes only
    /// without a backtick), a `- ` list, or a paragraph.
    fn block(&mut self, first: &'a str) -> Result<Block, MarkdownError> {
        match (first.strip_prefix("```"), first.strip_prefix("- ")) {
            (Some(info), _) if !info.contains('`') => self.code(info),
            (_, Some(_)) => self.list(),
            _ => self.paragraph(),
        }
    }

    fn paragraph(&mut self) -> Result<Block, MarkdownError> {
        let opened = self.line();
        let mut texts = vec![];
        while let Some(line) = self.peek(0).filter(|line| !line.is_empty()) {
            texts.push(self.text(line)?);
            self.at += 1;
        }
        // Refused read as written: a definition, escaped or not.
        match defines_reference(&texts) {
            true => Err(MarkdownError::Link { line: opened }),
            false => Ok(Block::Paragraph(texts)),
        }
    }

    fn list(&mut self) -> Result<Block, MarkdownError> {
        let mut items = vec![];
        while let Some(item) = self.peek(0).and_then(|line| line.strip_prefix("- ")) {
            items.push(self.text(item)?);
            self.at += 1;
        }
        Ok(Block::List(items))
    }

    /// A line of text: the backslash the writer adds taken off, and one a
    /// block would start, left unescaped, refused.
    fn text(&self, line: &str) -> Result<String, MarkdownError> {
        unescaped(line).ok_or(MarkdownError::BlockLike { line: self.line() })
    }

    /// Fenced code: its lines verbatim up to the closing fence.
    fn code(&mut self, info: &str) -> Result<Block, MarkdownError> {
        let opened = self.line();
        let language = match info {
            "" => None,
            name if language(name) => Some(name.to_string()),
            _ => {
                return Err(MarkdownError::Fence {
                    line: opened,
                    why: LANGUAGE_RULE,
                });
            }
        };
        self.at += 1;
        let mut lines = vec![];
        loop {
            match self.peek(0) {
                Some("```") => {
                    self.at += 1;
                    return Ok(Block::Code { language, lines });
                }
                // The writer refuses every line a reader could close on.
                Some(line) if line.trim_start_matches(' ').starts_with("```") => {
                    return Err(MarkdownError::Fence {
                        line: self.line(),
                        why: "a line a fence would end on",
                    });
                }
                Some(line) => {
                    lines.push(line.to_string());
                    self.at += 1;
                }
                None => {
                    return Err(MarkdownError::Fence {
                        line: opened,
                        why: "a fence left open",
                    });
                }
            }
        }
    }
}
