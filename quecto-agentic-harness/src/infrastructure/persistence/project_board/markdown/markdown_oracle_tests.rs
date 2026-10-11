//! The oracle: what a CommonMark/GFM reader makes of a file, block by
//! block, against the document it was rendered from.
//!
//! Two readers: pulldown-cmark with every extension on but four GitHub
//! does not have or reads otherwise, which only change what text shows
//! (smart punctuation, heading attributes `{#id}`, `^superscript^`, and
//! `~subscript~`, which GitHub draws as strikethrough); and comrak, a port
//! of cmark-gfm, with GFM's tables, strikethrough and extended autolinks
//! (a bare `www.`, `https://` or email), which pulldown-cmark does not read,
//! task lists, footnotes, math, alerts and the tag filter, as GitHub reads.
use super::*;
use comrak::nodes::{AstNode, ListType, NodeValue};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag};
use regex::Regex;

/// pulldown-cmark's options: every extension but the four named above.
fn options() -> Options {
    Options::all()
        - Options::ENABLE_SMART_PUNCTUATION
        - Options::ENABLE_HEADING_ATTRIBUTES
        - Options::ENABLE_SUPERSCRIPT
        - Options::ENABLE_SUBSCRIPT
}

/// comrak's options: what GitHub reads: autolinks, tables,
/// strikethrough, task lists, footnotes, `$` and code math, alerts, and
/// the tag filter.
fn comrak_options() -> comrak::Options<'static> {
    let mut options = comrak::Options::default();
    let extension = &mut options.extension;
    extension.autolink = true;
    extension.table = true;
    extension.strikethrough = true;
    extension.tasklist = true;
    extension.footnotes = true;
    extension.math_dollars = true;
    extension.math_code = true;
    extension.alerts = true;
    extension.tagfilter = true;
    options
}

/// A reader's outline: per top-level block its kind (with counts and
/// links) and the text it shows, then the text each link shows, then the
/// run of text around each link.
type Read = (Vec<String>, Vec<String>, Vec<String>, Vec<String>);

/// The run of non-space text around each link of `text` (each `range`), as
/// a reader sees it run together: what precedes it up to a space, its
/// text, and what follows it up to a space.
fn runs(text: &str, ranges: &[std::ops::Range<usize>]) -> Vec<String> {
    ranges
        .iter()
        .map(|range| {
            let start = text[..range.start]
                .rfind(char::is_whitespace)
                .map_or(0, |at| at + 1);
            let end = text[range.end..]
                .find(char::is_whitespace)
                .map_or(text.len(), |at| range.end + at);
            text[start..end].to_string()
        })
        .collect()
}

/// A link's address as a text writes it, after its `]`.
static ADDRESS: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"\]\(https://[^)\s]*\)").expect("a valid pattern"));

/// Whether `text` shows a reader something: a letter, number, punctuation
/// or symbol that is not an emphasis or strikethrough marker.
fn shows(text: &str) -> bool {
    text.chars().any(|c| chars::seen(c) && !"*_~".contains(c))
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

/// What a reader shows at each edge of a code span: so a code cell (whose
/// expected text carries it) must be shown as code, never as plain text.
const SPAN: char = '\u{1}';

/// `text` as a code span shows it, its edges marked.
fn spanned(text: &str) -> String {
    format!("{SPAN}{text}{SPAN}")
}

/// Whether `shown` is `written` with only markup taken out: emphasis and
/// strikethrough markers, a code span's backticks (each edge a reader
/// marks matching one) and padding, and a link's brackets and address.
fn drops_only_markup(written: &str, shown: &str) -> bool {
    let written = ADDRESS.replace_all(written, "]");
    let mut shown = shown.chars().peekable();
    for c in written.chars() {
        match (shown.peek().copied(), c) {
            (Some(next), c) if next == c => {
                shown.next();
            }
            (Some(SPAN), '`') => {
                shown.next();
            }
            (_, c) if "*_~`[] ".contains(c) => {}
            _ => return false,
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
    /// Links of the top-level block, and the text each shows.
    links: usize,
    link_texts: Vec<String>,
    /// Where each link of the top-level block starts and ends in `text`,
    /// and the runs of text around the links read so far.
    link_ranges: Vec<std::ops::Range<usize>>,
    link_runs: Vec<String>,
}

impl Outline {
    /// `text`, shown in the block (and in the link it is in, if any).
    fn shown(&mut self, text: &str) {
        self.text.push_str(text);
        if let (true, Some(link)) = (self.open.contains(&"link"), self.link_texts.last_mut()) {
            link.push_str(text);
        }
    }

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
            Tag::Emphasis | Tag::Strong | Tag::Strikethrough => "inline",
            Tag::Link {
                link_type: LinkType::Inline,
                dest_url,
                title,
                ..
            } if dest_url.starts_with("https://")
                && title.is_empty()
                && !open.contains(&"link") =>
            {
                "link"
            }
            other => return Err(format!("{other:?} in {open:?}")),
        };
        if self.open.is_empty() {
            (self.count, self.cells, self.width, self.ragged) = (0, 0, None, false);
            self.text.clear();
            self.links = 0;
        }
        match kind {
            "item" | "row" => self.count += 1,
            "cell" => self.cells += 1,
            "link" => {
                self.links += 1;
                self.link_texts.push(String::new());
                self.link_ranges.push(self.text.len()..self.text.len());
            }
            _ => {}
        }
        self.open.push(kind);
        Ok(())
    }

    fn end(&mut self) -> Result<(), String> {
        let closed = self.open.pop().ok_or("an end with nothing open")?;
        if let ("link", Some(range)) = (closed, self.link_ranges.last_mut()) {
            range.end = self.text.len();
        }
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
            let kind = match closed {
                "paragraph" => format!("paragraph:{}", self.count + 1),
                "list" => format!("list:{}", self.count),
                "code" => format!("code:{}", self.text.matches('\n').count()),
                "table" if self.ragged => "table:ragged".to_string(),
                "table" => format!("table:{}x{}", self.width.unwrap_or(0), self.count),
                other => other.to_string(),
            };
            self.kinds.push(format!("{kind}+{}", self.links));
            let ranges = std::mem::take(&mut self.link_ranges);
            self.link_runs.extend(runs(&self.text, &ranges));
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
                self.shown(text);
            }
            Event::Code(text) => {
                checked(text)?;
                self.shown(&spanned(text));
            }
            Event::Html(_) if self.kinds.is_empty() => {}
            // Emphasis may span a line, so a break may come inside it.
            Event::SoftBreak
                if self.open.first() == Some(&"paragraph") && !self.open.contains(&"link") =>
            {
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
    let (kinds, texts) = unlinked_expected(document);
    let kinds = kinds
        .into_iter()
        .zip(&texts)
        .map(|(kind, text)| {
            // Code is verbatim: what looks like a link there is none.
            let links = match kind.starts_with("code:") {
                true => 0,
                false => ADDRESS.find_iter(text).count(),
            };
            format!("{kind}+{links}")
        })
        .collect();
    (kinds, texts)
}

/// [`expected`] without the links each block holds.
fn unlinked_expected(document: &Document) -> (Vec<String>, Vec<String>) {
    let mut kinds = vec!["marker".to_string(), "h1".to_string()];
    let mut texts = vec![String::new(), document.title.clone()];
    if !document.fields.is_empty() {
        let rows: Vec<Vec<String>> = document
            .fields
            .iter()
            .map(|(name, value)| {
                let kind = SHAPE.fields.iter().find(|field| field.name == name);
                vec![
                    name.clone(),
                    shown_cell(value, kind.map(|field| field.cell)),
                ]
            })
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
                let rows: Vec<Vec<String>> = rows
                    .iter()
                    .map(|row| {
                        row.iter()
                            .zip(columns)
                            .map(|(value, column)| shown_cell(value, Some(column.cell)))
                            .collect()
                    })
                    .collect();
                texts.push(table_text(columns.iter().map(|column| column.name), &rows));
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
    let comrak = comrak_outline(file).map_err(|error| format!("comrak: {error}"))?;
    for (reader, shown) in [
        (
            "pulldown-cmark",
            &(
                outline.kinds,
                outline.texts,
                outline.link_texts,
                outline.link_runs,
            ),
        ),
        ("comrak", &comrak),
    ] {
        compared(reader, shown, &kinds, &texts)?;
    }
    Ok(())
}

/// One reader's outline (kinds, texts, link texts) against the document's.
fn compared(
    reader: &str,
    (shown_kinds, shown_texts, link_texts, link_runs): &Read,
    kinds: &[String],
    texts: &[String],
) -> Result<(), String> {
    if shown_kinds != kinds {
        return Err(format!("{reader}: {shown_kinds:?}, not {kinds:?}"));
    }
    for ((kind, shown), written) in kinds.iter().zip(shown_texts).zip(texts) {
        let same = match kind.starts_with("code:") {
            true => shown == written,
            false => drops_only_markup(written, shown),
        };
        if !same {
            return Err(format!(
                "{reader}: {kind} shows {shown:?}, written {written:?}"
            ));
        }
        // Each title, cell, paragraph line and list item shows something.
        let units: Vec<&str> = match kind.split([':', '+']).next() {
            Some("h1") => vec![shown.as_str()],
            Some("paragraph" | "list") => shown.lines().collect(),
            Some("table") => shown.lines().flat_map(|row| row.split('\t')).collect(),
            _ => vec![],
        };
        if let Some(blank) = units.iter().find(|unit| !shows(unit)) {
            return Err(format!("{reader}: {kind} shows nothing in {blank:?}"));
        }
    }
    if let Some(blank) = link_texts.iter().find(|text| !shows(text)) {
        return Err(format!("{reader}: a link shows nothing: {blank:?}"));
    }
    // A link's text, and the run of text around it, shows no address.
    match link_texts
        .iter()
        .chain(link_runs)
        .find(|text| shows_address(text))
    {
        Some(address) => Err(format!("{reader}: a link shows an address: {address:?}")),
        None => Ok(()),
    }
}

/// Whether a link's shown text (code contents included) shows an address
/// a reader would take for its own: a scheme, `www.`, `mailto:`, `xmpp:` or
/// an email.
fn shows_address(text: &str) -> bool {
    static EMAIL: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"[A-Za-z0-9.+_-]@").expect("a valid pattern"));
    static MARK: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"\p{M}").expect("a valid pattern"));
    // A combining mark inside an address hides it from a pattern, not a reader.
    let text = MARK.replace_all(text, "");
    let lower = text.to_ascii_lowercase();
    ["://", "www.", "mailto:", "xmpp:"]
        .iter()
        .any(|form| lower.contains(form))
        || EMAIL.is_match(&text)
}

/// The inline text `node` shows, and where each link in it starts and
/// ends in that text (each link's text added to `link_texts`). Only text,
/// code, emphasis, strikethrough, soft breaks (never in a link) and inline
/// https links with no title are allowed.
fn comrak_inline<'a>(
    node: &'a AstNode<'a>,
    link_texts: &mut Vec<String>,
    in_link: bool,
) -> Result<(String, Vec<std::ops::Range<usize>>), String> {
    let (mut text, mut links) = (String::new(), vec![]);
    for child in node.children() {
        match &child.data.borrow().value {
            NodeValue::Text(shown) => text.push_str(shown),
            NodeValue::Code(code) => text.push_str(&spanned(&code.literal)),
            NodeValue::SoftBreak if !in_link => text.push('\n'),
            NodeValue::Emph | NodeValue::Strong | NodeValue::Strikethrough => {
                let (shown, inner) = comrak_inline(child, link_texts, in_link)?;
                let at = text.len();
                links.extend(
                    inner
                        .into_iter()
                        .map(|range| range.start + at..range.end + at),
                );
                text.push_str(&shown);
            }
            NodeValue::Link(link)
                if link.url.starts_with("https://") && link.title.is_empty() && !in_link =>
            {
                let (shown, _) = comrak_inline(child, &mut vec![], true)?;
                link_texts.push(shown.clone());
                links.push(text.len()..text.len() + shown.len());
                text.push_str(&shown);
            }
            other => return Err(format!("{other:?}")),
        }
    }
    Ok((text, links))
}

/// comrak's outline of `file`, block by block, as [`Outline`] keeps it.
fn comrak_outline(file: &str) -> Result<Read, String> {
    let arena = comrak::Arena::new();
    let root = comrak::parse_document(&arena, file, &comrak_options());
    let (mut kinds, mut texts, mut link_texts, mut link_runs) = (vec![], vec![], vec![], vec![]);
    // The inline text of `node`, and how many links it holds, the runs of
    // text around them kept.
    let mut inline = |node, link_texts: &mut Vec<String>| -> Result<(String, usize), String> {
        let (text, ranges) = comrak_inline(node, link_texts, false)?;
        link_runs.extend(runs(&text, &ranges));
        Ok((text, ranges.len()))
    };
    for block in root.children() {
        let (kind, text, links) = match &block.data.borrow().value {
            NodeValue::HtmlBlock(_) if kinds.is_empty() => ("marker".to_string(), String::new(), 0),
            NodeValue::Heading(heading) if heading.level == 1 && kinds.len() == 1 => {
                let (text, links) = inline(block, &mut link_texts)?;
                ("h1".to_string(), text, links)
            }
            NodeValue::Heading(heading) if heading.level == 2 => {
                let (text, links) = inline(block, &mut link_texts)?;
                ("h2".to_string(), text, links)
            }
            NodeValue::Paragraph => {
                let (text, links) = inline(block, &mut link_texts)?;
                (
                    format!("paragraph:{}", text.matches('\n').count() + 1),
                    text,
                    links,
                )
            }
            NodeValue::List(list) if list.list_type == ListType::Bullet => {
                let (mut text, mut links, mut items) = (String::new(), 0, 0);
                for item in block.children() {
                    let paragraphs: Vec<_> = item.children().collect();
                    match (&item.data.borrow().value, paragraphs.as_slice()) {
                        (NodeValue::Item(_), [paragraph])
                            if matches!(paragraph.data.borrow().value, NodeValue::Paragraph) =>
                        {
                            let (shown, inner) = inline(paragraph, &mut link_texts)?;
                            text += &format!("{shown}\n");
                            links += inner;
                            items += 1;
                        }
                        (other, _) => return Err(format!("{other:?} in a list")),
                    }
                }
                (format!("list:{items}"), text, links)
            }
            NodeValue::CodeBlock(code) if code.fenced => (
                format!("code:{}", code.literal.matches('\n').count()),
                code.literal.clone(),
                0,
            ),
            NodeValue::Table(_) => comrak_table(block, &mut link_texts, &mut inline)?,
            other => return Err(format!("{other:?}")),
        };
        kinds.push(format!("{kind}+{links}"));
        texts.push(text);
    }
    Ok((kinds, texts, link_texts, link_runs))
}

/// A table's kind, text and links, as [`Outline`] keeps them.
fn comrak_table<'a>(
    table: &'a AstNode<'a>,
    link_texts: &mut Vec<String>,
    inline: &mut impl FnMut(&'a AstNode<'a>, &mut Vec<String>) -> Result<(String, usize), String>,
) -> Result<(String, String, usize), String> {
    let (mut text, mut links, mut rows, mut width, mut ragged) = (String::new(), 0, 0, None, false);
    for row in table.children() {
        let header = match &row.data.borrow().value {
            NodeValue::TableRow(header) => *header,
            other => return Err(format!("{other:?} in a table")),
        };
        let mut cells = vec![];
        for cell in row.children() {
            let (shown, inner) = inline(cell, link_texts)?;
            cells.push(shown);
            links += inner;
        }
        match width {
            None => width = Some(cells.len()),
            Some(header) => ragged |= header != cells.len(),
        }
        rows += usize::from(!header);
        text += &(cells.join("\t") + "\n");
    }
    let kind = match ragged {
        true => "table:ragged".to_string(),
        false => format!("table:{}x{rows}", width.unwrap_or(0)),
    };
    Ok((kind, text, links))
}

/// A cell's text as a reader must show it: a code cell as a code span.
fn shown_cell(value: &str, kind: Option<CellKind>) -> String {
    match kind {
        Some(CellKind::Code) => spanned(value),
        Some(CellKind::Text | CellKind::Ascii) | None => value.to_string(),
    }
}

/// A code cell shown as plain text is caught: its expected text is a span.
#[test]
fn a_code_cell_a_reader_shows_as_text_is_caught() {
    let document = Document {
        schema: 1,
        title: "Code".into(),
        fields: vec![("Branch".into(), "main".into())],
        sections: vec![],
    };
    let file = render(&document, &SHAPE).unwrap();
    assert_eq!(outline(&file, &document), Ok(()));
    let plain = file.replacen("`main`", "main", 1);
    assert!(outline(&plain, &document).is_err(), "{plain}");
}
