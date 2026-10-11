//! The oracle: what a CommonMark/GFM reader makes of a file, block by
//! block, against the document it was rendered from.
//!
//! The reader is pulldown-cmark with every extension on but three GitHub
//! does not have, which only change what text shows: smart punctuation,
//! heading attributes (`{#id}`) and `^superscript^`.
use super::*;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag};
use regex::Regex;

/// The reader's options: every extension but the three named above.
fn options() -> Options {
    Options::all()
        - Options::ENABLE_SMART_PUNCTUATION
        - Options::ENABLE_HEADING_ATTRIBUTES
        - Options::ENABLE_SUPERSCRIPT
}

/// A text's characters, checked as a run (a reader splits text at a `*`,
/// say); a run may end before inline markup with a space.
fn checked(text: &str) -> Result<(), String> {
    text.split('\n')
        .map(|line| line.trim_end_matches(' '))
        .try_for_each(|line| {
            chars::check_line(line, 0).map_err(|error| format!("{line:?}: {error:?}"))
        })
}

/// Whether `shown` is `written` with only markup taken out: emphasis and
/// strikethrough markers, a code span's backticks and padding, and a
/// link's brackets and address.
fn drops_only_markup(written: &str, shown: &str) -> bool {
    static ADDRESS: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"\]\(https://[^)\s]*\)").expect("a valid pattern"));
    let written = ADDRESS.replace_all(written, "]");
    let mut shown = shown.chars().peekable();
    for c in written.chars() {
        match shown.peek() == Some(&c) {
            true => {
                shown.next();
            }
            false if "*_~`[] ".contains(c) => {}
            false => return false,
        }
    }
    shown.next().is_none()
}

/// The reader's outline of a file: per top-level block its kind and
/// counts, and the text it shows.
#[derive(Default)]
struct Outline {
    kinds: Vec<String>,
    texts: Vec<String>,
    /// Open tags, outermost first.
    open: Vec<&'static str>,
    /// Lines, items or rows of the top-level block; cells of the row.
    count: usize,
    cells: usize,
    /// The header's width, and whether any row is of another.
    width: Option<usize>,
    ragged: bool,
    text: String,
    run: String,
}

impl Outline {
    fn start(&mut self, tag: &Tag<'_>) -> Result<(), String> {
        let open = self.open.as_slice();
        let kind = match tag {
            Tag::HtmlBlock if self.kinds.is_empty() => "marker",
            Tag::Heading {
                level: HeadingLevel::H1,
                ..
            } if self.kinds.len() == 1 => "h1",
            Tag::Heading {
                level: HeadingLevel::H2,
                ..
            } if open.is_empty() => "h2",
            Tag::Paragraph if open.is_empty() => "paragraph",
            Tag::List(None) if open.is_empty() => "list",
            Tag::Item if open == ["list"] => "item",
            Tag::CodeBlock(CodeBlockKind::Fenced(_)) if open.is_empty() => "code",
            Tag::Table(_) if open.is_empty() => "table",
            Tag::TableHead if open == ["table"] => "head",
            Tag::TableRow if open == ["table"] => "row",
            Tag::TableCell if matches!(open, ["table", "head" | "row"]) => "cell",
            Tag::Emphasis | Tag::Strong | Tag::Strikethrough | Tag::Subscript => "inline",
            Tag::Link {
                link_type: LinkType::Inline,
                dest_url,
                title,
                ..
            } if dest_url.starts_with("https://") && title.is_empty() => "inline",
            other => return Err(format!("{other:?} in {open:?}")),
        };
        if self.open.is_empty() {
            (self.count, self.cells, self.width, self.ragged) = (0, 0, None, false);
            self.text.clear();
        }
        match kind {
            "item" | "row" => self.count += 1,
            "cell" => self.cells += 1,
            _ => {}
        }
        self.open.push(kind);
        Ok(())
    }

    fn end(&mut self) -> Result<(), String> {
        let closed = self.open.pop().ok_or("an end with nothing open")?;
        match closed {
            "item" => self.text.push('\n'),
            "cell" => self.text.push('\t'),
            "head" | "row" => {
                match self.width {
                    None => self.width = Some(self.cells),
                    Some(header) => self.ragged |= header != self.cells,
                }
                self.cells = 0;
                if self.text.ends_with('\t') {
                    self.text.pop();
                }
                self.text.push('\n');
            }
            _ => {}
        }
        if self.open.is_empty() {
            self.kinds.push(match closed {
                "paragraph" => format!("paragraph:{}", self.count + 1),
                "list" => format!("list:{}", self.count),
                "code" => format!("code:{}", self.text.matches('\n').count()),
                "table" if self.ragged => "table:ragged".to_string(),
                "table" => format!("table:{}x{}", self.width.unwrap_or(0), self.count),
                other => other.to_string(),
            });
            self.texts.push(std::mem::take(&mut self.text));
        }
        Ok(())
    }

    fn event(&mut self, event: &Event<'_>) -> Result<(), String> {
        if !matches!(event, Event::Text(_)) {
            checked(&std::mem::take(&mut self.run))?;
        }
        match event {
            Event::Start(tag) => self.start(tag)?,
            Event::End(_) => self.end()?,
            Event::Text(text) => {
                self.run.push_str(text);
                self.text.push_str(text);
            }
            Event::Code(text) => {
                checked(text)?;
                self.text.push_str(text);
            }
            Event::Html(_) if self.kinds.is_empty() => {}
            Event::SoftBreak if self.open == ["paragraph"] => {
                self.count += 1;
                self.text.push('\n');
            }
            other => return Err(format!("{other:?} in {:?}", self.open)),
        }
        Ok(())
    }
}

/// A table's text as the outline keeps it: a line a row, tab-separated.
fn table_text<'a>(header: impl Iterator<Item = &'a str>, rows: &[Vec<String>]) -> String {
    let mut text = header.collect::<Vec<_>>().join("\t") + "\n";
    for row in rows {
        text += &(row.join("\t") + "\n");
    }
    text
}

/// `document`'s outline: per top-level block its kind and counts, and its
/// text as written.
fn expected(document: &Document) -> (Vec<String>, Vec<String>) {
    let mut kinds = vec!["marker".to_string(), "h1".to_string()];
    let mut texts = vec![String::new(), document.title.clone()];
    if !document.fields.is_empty() {
        let rows: Vec<Vec<String>> = document
            .fields
            .iter()
            .map(|(name, value)| vec![name.clone(), value.clone()])
            .collect();
        kinds.push(format!("table:2x{}", rows.len()));
        texts.push(table_text(["Field", "Value"].into_iter(), &rows));
    }
    for section in &document.sections {
        kinds.push("h2".into());
        texts.push(section.heading.clone());
        match &section.body {
            Body::Prose(blocks) => {
                for block in blocks {
                    let (kind, text) = match block {
                        Block::Paragraph(lines) => {
                            (format!("paragraph:{}", lines.len()), lines.join("\n"))
                        }
                        Block::List(items) => (
                            format!("list:{}", items.len()),
                            items.iter().map(|item| format!("{item}\n")).collect(),
                        ),
                        Block::Code { lines, .. } => (
                            format!("code:{}", lines.len()),
                            lines.iter().map(|line| format!("{line}\n")).collect(),
                        ),
                    };
                    kinds.push(kind);
                    texts.push(text);
                }
            }
            Body::Table(rows) => {
                let columns = SHAPE
                    .sections
                    .iter()
                    .find(|known| known.heading == section.heading)
                    .map_or(&[][..], |known| match known.body {
                        BodyShape::Table(columns) => columns,
                        BodyShape::Prose => &[],
                    });
                kinds.push(format!("table:{}x{}", columns.len(), rows.len()));
                texts.push(table_text(columns.iter().map(|column| column.name), rows));
            }
        }
    }
    (kinds, texts)
}

/// What a CommonMark/GFM reader makes of `file` is exactly the outline
/// `document` has, block by block: the marker, one H1, the field table,
/// then per section one H2 and its paragraphs (of as many lines), one-level
/// `- ` lists (of as many items, nothing but inline text in one), fenced
/// code (of as many lines) and tables (as many rows, under a header as wide
/// as the shape's columns; a reader pads or cuts each row to the header, so
/// a row's own width is the renderer's to keep). No HTML, math, image,
/// footnote, quote, rule, hard break, ordered or nested list, or other link
/// than an inline https one. Every text it shows keeps the character rules,
/// and is the document's own with only markup taken out (code exactly).
pub(super) fn outline(file: &str, document: &Document) -> Result<(), String> {
    let mut outline = Outline::default();
    for event in Parser::new_ext(file, options()) {
        outline.event(&event)?;
    }
    checked(&outline.run)?;
    let (kinds, texts) = expected(document);
    if outline.kinds != kinds {
        return Err(format!("{:?}, not {kinds:?}", outline.kinds));
    }
    for ((kind, shown), written) in kinds.iter().zip(&outline.texts).zip(&texts) {
        let same = match kind.starts_with("code:") {
            true => shown == written,
            false => drops_only_markup(written, shown),
        };
        if !same {
            return Err(format!("{kind} shows {shown:?}, written {written:?}"));
        }
    }
    Ok(())
}
