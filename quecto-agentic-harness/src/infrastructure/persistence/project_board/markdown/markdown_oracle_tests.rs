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
use comrak_reader::comrak_outline;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag};
use regex::Regex;

#[path = "markdown_oracle_comrak_tests.rs"]
mod comrak_reader;

/// pulldown-cmark's options: every extension but the four named above.
fn options() -> Options {
    Options::all()
        - Options::ENABLE_SMART_PUNCTUATION
        - Options::ENABLE_HEADING_ATTRIBUTES
        - Options::ENABLE_SUPERSCRIPT
        - Options::ENABLE_SUBSCRIPT
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
/// or symbol that is not an emphasis or strikethrough marker, or anything
/// seen inside a code span (between the marks a reader leaves), where a
/// marker shows as itself.
fn shows(text: &str) -> bool {
    text.split(SPAN)
        .enumerate()
        .any(|(index, part)| match index % 2 {
            1 => part.chars().any(chars::seen),
            _ => part.chars().any(|c| chars::seen(c) && !"*_~".contains(c)),
        })
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

/// `written` in pieces: text, and code (each code span's content as a
/// reader shows it: one space at each edge taken off when both are
/// there; a value the expected outline already marks as a span, kept).
fn pieces(written: &str) -> Vec<(bool, String)> {
    let (mut pieces, mut text) = (vec![], String::new());
    let mut rest = written;
    while let Some(c) = rest.chars().next() {
        let span = match c {
            SPAN => rest[SPAN.len_utf8()..].find(SPAN).map(|end| {
                (
                    rest[SPAN.len_utf8()..][..end].to_string(),
                    2 * SPAN.len_utf8() + end,
                )
            }),
            '`' => {
                let run = rest.len() - rest.trim_start_matches('`').len();
                let after = &rest[run..];
                let fence = "`".repeat(run);
                // The next run of exactly as many backticks closes it.
                let close = after.match_indices(&fence).find(|(at, _)| {
                    !after[..*at].ends_with('`') && !after[at + run..].starts_with('`')
                });
                close.map(|(at, _)| {
                    let inner = &after[..at];
                    let padded =
                        inner.starts_with(' ') && inner.ends_with(' ') && inner.trim() != "";
                    let shown = match padded {
                        true => &inner[1..inner.len() - 1],
                        false => inner,
                    };
                    (shown.to_string(), 2 * run + at)
                })
            }
            _ => None,
        };
        match span {
            Some((code, taken)) => {
                pieces.push((false, std::mem::take(&mut text)));
                pieces.push((true, code));
                rest = &rest[taken..];
            }
            None => {
                text.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    pieces.push((false, text));
    pieces
}

/// The links `written` holds outside its code spans.
fn links_outside_code(written: &str) -> usize {
    pieces(written)
        .iter()
        .filter(|(code, _)| !code)
        .map(|(_, text)| ADDRESS.find_iter(text).count())
        .sum()
}

/// Whether `shown` is `written` with only markup taken out: outside code,
/// emphasis and strikethrough markers, and a link's brackets and address;
/// a code span's text exactly, between the edges a reader marks.
fn drops_only_markup(written: &str, shown: &str) -> bool {
    let mut shown = shown.chars().peekable();
    for (code, piece) in pieces(written) {
        match code {
            true => {
                let exact = std::iter::once(SPAN)
                    .chain(piece.chars())
                    .chain(std::iter::once(SPAN));
                for c in exact {
                    match shown.next() {
                        Some(next) if next == c => {}
                        _ => return false,
                    }
                }
            }
            false => {
                for c in ADDRESS.replace_all(&piece, "]").chars() {
                    match shown.peek().copied() {
                        Some(next) if next == c => {
                            shown.next();
                        }
                        _ if "*_~[] ".contains(c) => {}
                        _ => return false,
                    }
                }
            }
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
fn expected(document: &Document, shape: &Shape<'_>) -> (Vec<String>, Vec<String>) {
    let (kinds, texts) = unlinked_expected(document, shape);
    let kinds = kinds
        .into_iter()
        .zip(&texts)
        .map(|(kind, text)| {
            // Code is verbatim: what looks like a link there is none.
            let links = match kind.starts_with("code:") {
                true => 0,
                false => links_outside_code(text),
            };
            format!("{kind}+{links}")
        })
        .collect();
    (kinds, texts)
}

/// [`expected`] without the links each block holds.
fn unlinked_expected(document: &Document, shape: &Shape<'_>) -> (Vec<String>, Vec<String>) {
    let mut kinds = vec!["marker".to_string(), "h1".to_string()];
    let mut texts = vec![String::new(), document.title.clone()];
    if !document.fields.is_empty() {
        let rows: Vec<Vec<String>> = document
            .fields
            .iter()
            .map(|(name, value)| {
                let kind = shape.fields.iter().find(|field| field.name == name);
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
                let columns = shape
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
/// Codec tests use it to check every cell of the files they write.
pub(crate) fn outline_in(file: &str, document: &Document, shape: &Shape<'_>) -> Result<(), String> {
    let mut outline = Outline::default();
    for event in Parser::new_ext(file, options()) {
        outline.event(&event)?;
    }
    checked(&outline.run)?;
    let (kinds, texts) = expected(document, shape);
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

/// Inside a code span the text is compared exactly: no markup is taken
/// out there, so a reader showing `ab` for a written `a*b` is caught.
#[test]
fn a_code_span_is_compared_exactly() {
    let document = Document {
        schema: 1,
        title: "Code".into(),
        fields: vec![("Branch".into(), "a*b_c[d]".into())],
        sections: vec![],
    };
    let file = render(&document, &SHAPE).unwrap();
    assert_eq!(outline(&file, &document), Ok(()));
    for shown in ["`ab_c[d]`", "`a*bc[d]`", "`a*b_cd]`", "`a*b_c[d`"] {
        let edited = file.replacen("`a*b_c[d]`", shown, 1);
        assert!(outline(&edited, &document).is_err(), "{edited}");
    }
}

/// What looks like a link inside a code span is code: no address is taken
/// out and no link is counted, by the oracle or by a reader.
#[test]
fn a_code_value_that_looks_like_a_link_is_code() {
    let document = Document {
        schema: 1,
        title: "Code".into(),
        fields: vec![
            ("Claim".into(), "see `[x](https://y.example)` here".into()),
            ("Branch".into(), "[x](https://y.example)".into()),
        ],
        sections: vec![],
    };
    let file = render(&document, &SHAPE).unwrap();
    assert_eq!(outline(&file, &document), Ok(()), "{file}");
}

/// A code span shows what it holds, a lone marker included: a reader sees
/// the `_` in `` [`_`](…) ``, so the link shows something.
#[test]
fn a_code_span_of_a_marker_shows_it() {
    for claim in ["[`_`](https://y.example/a)", "see `*` and `~~`"] {
        let document = Document {
            schema: 1,
            title: "Code".into(),
            fields: vec![("Claim".into(), claim.into())],
            sections: vec![],
        };
        let file = render(&document, &SHAPE).unwrap();
        assert_eq!(outline(&file, &document), Ok(()), "{file}");
    }
}
