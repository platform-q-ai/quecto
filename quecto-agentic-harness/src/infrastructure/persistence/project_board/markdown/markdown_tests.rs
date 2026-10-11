//! The codec over well-formed documents: an exact file, generated
//! documents read back by a CommonMark/GFM reader (the oracle), block
//! starts kept as text, and the limits.
use super::*;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, LinkType, Options, Parser, Tag};

const FIELDS: &[Column<'static>] = &[
    Column {
        name: "Id",
        cell: CellKind::Ascii,
    },
    Column {
        name: "Status",
        cell: CellKind::Ascii,
    },
    Column {
        name: "Claim",
        cell: CellKind::Text,
    },
];

const ITEMS: &[Column<'static>] = &[
    Column {
        name: "Item",
        cell: CellKind::Text,
    },
    Column {
        name: "State",
        cell: CellKind::Ascii,
    },
];

const SECTIONS: &[SectionShape<'static>] = &[
    SectionShape {
        heading: "Outcome",
        body: BodyShape::Prose,
    },
    SectionShape {
        heading: "Acceptance",
        body: BodyShape::Prose,
    },
    SectionShape {
        heading: "Items",
        body: BodyShape::Table(ITEMS),
    },
];

pub(super) const LIMITS: Limits = Limits {
    file_bytes: 1 << 20,
    lines: 8_192,
    line_bytes: 4_096,
};

pub(super) const SHAPE: Shape<'static> = Shape {
    kind: "task",
    fields: FIELDS,
    sections: SECTIONS,
    limits: LIMITS,
};

fn texts(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|text| (*text).to_string()).collect()
}

pub(super) fn example() -> Document {
    Document {
        schema: 1,
        title: "Boards S2a: quecto board CLI".into(),
        fields: vec![
            ("Id".into(), "board-cli".into()),
            ("Status".into(), "review".into()),
            (
                "Claim".into(),
                "Ada (ada@example.com) until 2026-10-11T14:00:00Z".into(),
            ),
        ],
        sections: vec![
            Section {
                heading: "Outcome".into(),
                body: Body::Prose(vec![
                    Block::Paragraph(texts(&[
                        "Agents read the board, `Vec<T>` and all.",
                        "# not a heading",
                        "1. not a list",
                    ])),
                    Block::List(texts(&[
                        "read",
                        "write [the docs](https://example.com/docs)",
                    ])),
                    Block::Code {
                        language: Some("rust".into()),
                        lines: texts(&["fn main() {", "    let x = 1;", "}"]),
                    },
                ]),
            },
            Section {
                heading: "Acceptance".into(),
                body: Body::Prose(vec![Block::List(texts(&[
                    "Every op answers one JSON line",
                ]))]),
            },
            Section {
                heading: "Items".into(),
                body: Body::Table(vec![
                    texts(&["parse `a|b`", "done"]),
                    texts(&["path `C:\\x`", "open"]),
                ]),
            },
        ],
    }
}

pub(super) const EXAMPLE: &str = r##"<!-- quecto-task schema=1 -->
# Boards S2a: quecto board CLI

| Field | Value |
|---|---|
| Id | board-cli |
| Status | review |
| Claim | Ada (ada@example.com) until 2026-10-11T14:00:00Z |

## Outcome
Agents read the board, `Vec<T>` and all.
\# not a heading
1\. not a list

- read
- write [the docs](https://example.com/docs)

```rust
fn main() {
    let x = 1;
}
```

## Acceptance
- Every op answers one JSON line

## Items
| Item | State |
|---|---|
| parse `a\|b` | done |
| path `C:\x` | open |
"##;

/// What a CommonMark/GFM reader (every extension on) makes of `file` is
/// exactly the outline `document` has, block by block: the marker, one H1,
/// the field table, then per section one H2 and its paragraphs (of as many
/// lines), one-level `- ` lists (of as many items, nothing but inline text
/// in one), fenced code (of as many lines) and tables (as many rows, each
/// as wide as the header). No HTML, math, image, footnote, quote, rule,
/// hard break, ordered or nested list, or other link than an inline https
/// one; and every text it decodes keeps the character rules.
pub(super) fn outline(file: &str, document: &Document) -> Result<(), String> {
    let mut top: Vec<String> = vec![];
    // Open tags, outermost first; and the counts of the top-level block.
    let mut open: Vec<&'static str> = vec![];
    let (mut count, mut cells, mut code) = (0usize, 0usize, String::new());
    // The header's width, and whether any row is of another.
    let (mut width, mut ragged) = (None, false);
    for event in Parser::new_ext(file, Options::all()) {
        match &event {
            Event::Start(tag) => {
                let kind = match tag {
                    Tag::HtmlBlock if top.is_empty() => "marker",
                    Tag::Heading {
                        level: HeadingLevel::H1,
                        ..
                    } if top.len() == 1 => "h1",
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
                    Tag::TableCell if matches!(open.as_slice(), ["table", "head" | "row"]) => {
                        "cell"
                    }
                    Tag::Emphasis | Tag::Strong | Tag::Strikethrough => "inline",
                    Tag::Superscript | Tag::Subscript => "inline",
                    Tag::Link {
                        link_type: LinkType::Inline,
                        dest_url,
                        title,
                        ..
                    } if dest_url.starts_with("https://") && title.is_empty() => "inline",
                    other => return Err(format!("{other:?} in {open:?}")),
                };
                if open.is_empty() {
                    (count, cells, width, ragged) = (0, 0, None, false);
                    code.clear();
                }
                match kind {
                    "item" | "row" => count += 1,
                    "cell" => cells += 1,
                    _ => {}
                }
                open.push(kind);
            }
            Event::End(_) => {
                let closed = open.pop().ok_or("an end with nothing open")?;
                if let "head" | "row" = closed {
                    match width {
                        None => width = Some(cells),
                        Some(header) => ragged |= header != cells,
                    }
                    cells = 0;
                }
                if open.is_empty() {
                    top.push(match closed {
                        "paragraph" => format!("paragraph:{}", count + 1),
                        "list" => format!("list:{count}"),
                        "code" => format!("code:{}", code.matches('\n').count()),
                        "table" if ragged => "table:ragged".to_string(),
                        "table" => format!("table:{}x{count}", width.unwrap_or(0)),
                        other => other.to_string(),
                    });
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if open.first() == Some(&"code") {
                    code.push_str(text);
                }
                // A fragment may end before inline markup with a space: the
                // characters are what is checked here.
                for line in text.split('\n').map(|line| line.trim_end_matches(' ')) {
                    chars::check_line(line, 0).map_err(|error| format!("{line:?}: {error:?}"))?;
                }
            }
            Event::Html(_) if top.is_empty() => {}
            Event::SoftBreak if open == ["paragraph"] => count += 1,
            other => return Err(format!("{other:?} in {open:?}")),
        }
    }
    let mut expected = vec!["marker".to_string(), "h1".to_string()];
    if !document.fields.is_empty() {
        expected.push(format!("table:2x{}", document.fields.len()));
    }
    for section in &document.sections {
        expected.push("h2".into());
        match &section.body {
            Body::Prose(blocks) => expected.extend(blocks.iter().map(|block| match block {
                Block::Paragraph(lines) => format!("paragraph:{}", lines.len()),
                Block::List(items) => format!("list:{}", items.len()),
                Block::Code { lines, .. } => format!("code:{}", lines.len()),
            })),
            Body::Table(rows) => {
                let columns = SHAPE
                    .sections
                    .iter()
                    .find(|known| known.heading == section.heading)
                    .map_or(0, |known| match known.body {
                        BodyShape::Table(columns) => columns.len(),
                        BodyShape::Prose => 0,
                    });
                expected.push(format!("table:{columns}x{}", rows.len()));
            }
        }
    }
    match top == expected {
        true => Ok(()),
        false => Err(format!("{top:?}, not {expected:?}")),
    }
}

#[test]
fn a_document_renders_to_one_exact_file_a_reader_outlines_as_it() {
    assert_eq!(render(&example(), &SHAPE).as_deref(), Ok(EXAMPLE));
    assert_eq!(outline(EXAMPLE, &example()), Ok(()));
}

/// A small, seeded generator: the same documents on every run.
pub(super) struct Seeded(pub(super) u64);

impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % (n as u64)).unwrap_or(0)
    }

    fn pick(&mut self, pieces: &[&str]) -> String {
        pieces[self.below(pieces.len())].to_string()
    }

    /// A line: pieces that start blocks, escape and stretch Unicode, now
    /// and then a hostile one, now and then a leading space.
    pub(super) fn line(&mut self) -> String {
        #[rustfmt::skip]
        const SAFE: &[&str] = &[
            "a", "Z", "9", "1.", "2)", "#", "##", "|", ">", "=", "-", "---", "+", "*", "_", ":",
            "[x]", "<", "\\", "`code`", "`Vec<T>`", "~~~", "~", "é", "日本", "🦀", "👩🏽‍💻",
            "👨‍👩‍👧", "❤️", " ", "!", "(", ")", "\"", "'", ".", ",", "{", "}", "&", "%", "$ ",
            "[a link](https://example.com/x)", "x\u{301}\u{302}",
        ];
        #[rustfmt::skip]
        const HOSTILE: &[&str] = &[
            "\u{1}", "\u{202e}", "\u{200b}", "\u{fe0f}", "e\u{301}", "&amp;", "&#65;", "$x$",
            "![i](https://example.com/i.png)", "<b>", "http://x", "www.x", "\u{2800}", "\u{3164}",
            "a\u{fe0f}\u{200d}🦀", "\u{a0}", "[r][ref]", "[[w]]", "x\u{301}\u{302}\u{303}\u{304}\u{305}",
        ];
        let mut line: String = (0..1 + self.below(6)).map(|_| self.pick(SAFE)).collect();
        line = line.trim().to_string();
        if self.below(25) == 0 {
            line.push_str(&self.pick(HOSTILE));
        }
        if self.below(40) == 0 {
            line.insert(0, ' ');
        }
        line
    }

    fn ascii(&mut self) -> String {
        const ASCII: &[&str] = &["a", "z", "0", "9", ".", "_", ":", "/", "#", "@", "+", "-"];
        let mut cell: String = (0..1 + self.below(8)).map(|_| self.pick(ASCII)).collect();
        if self.below(40) == 0 {
            cell.push_str(&self.pick(&[" ", "é", "|"]));
        }
        cell
    }

    fn texts(&mut self) -> Vec<String> {
        (0..1 + self.below(3)).map(|_| self.line()).collect()
    }

    fn block(&mut self) -> Block {
        match self.below(3) {
            0 => Block::Paragraph(self.texts()),
            1 => Block::List(self.texts()),
            _ => Block::Code {
                language: [None, Some("rust".to_string()), Some("C++".to_string())][self.below(3)]
                    .clone(),
                // Now and then a line a reader would close the fence on, or
                // one at column 0 that would start a block outside code.
                lines: (0..self.below(3))
                    .map(|_| match self.below(30) {
                        0 => " ```".to_string(),
                        1..=6 => self.pick(&["", "## Outcome", "- x", "# x", "| a |", "1. x"]),
                        _ => format!("  {}", self.line()),
                    })
                    .collect(),
            },
        }
    }

    fn cell(&mut self, kind: CellKind) -> String {
        match kind {
            CellKind::Text => self.line(),
            CellKind::Ascii => self.ascii(),
        }
    }

    pub(super) fn document(&mut self) -> Document {
        let mut fields = vec![];
        for field in SHAPE.fields {
            if self.below(3) > 0 {
                fields.push((field.name.to_string(), self.cell(field.cell)));
            }
        }
        let mut sections = vec![];
        for section in SHAPE.sections {
            if self.below(3) == 0 {
                continue;
            }
            let body = match section.body {
                BodyShape::Prose => {
                    Body::Prose((0..1 + self.below(3)).map(|_| self.block()).collect())
                }
                BodyShape::Table(columns) => Body::Table(
                    (0..self.below(3))
                        .map(|_| {
                            columns
                                .iter()
                                .map(|column| self.cell(column.cell))
                                .collect()
                        })
                        .collect(),
                ),
            };
            sections.push(Section {
                heading: section.heading.into(),
                body,
            });
        }
        Document {
            schema: u32::try_from(self.below(1_000)).unwrap_or(0),
            title: self.line().trim_end_matches('#').trim().to_string(),
            fields,
            sections,
        }
    }
}

/// Every generated document the renderer takes, a CommonMark/GFM reader
/// outlines as the document; a fair share are taken, and a fair share
/// refused.
#[test]
fn every_rendered_document_is_outlined_as_itself() {
    let mut seeded = Seeded(0x9e37_79b9_7f4a_7c15);
    let (mut taken, mut refused) = (0, 0);
    for _ in 0..3_000 {
        let document = seeded.document();
        match render(&document, &SHAPE) {
            Ok(file) => {
                taken += 1;
                assert_eq!(outline(&file, &document), Ok(()), "{file}");
            }
            Err(_) => refused += 1,
        }
    }
    assert!(
        taken > 300 && refused > 300,
        "{taken} taken, {refused} refused"
    );
}

/// A paragraph line or list item that starts with any block start, or as
/// an ordered list item, reads as text in a paragraph or a list.
#[test]
fn block_starts_read_as_text() {
    let starts = prose::BLOCK_STARTS
        .chars()
        .map(|c| match c {
            '`' => "`a` x".to_string(),
            '$' => "$ x".to_string(),
            '<' => "< x".to_string(),
            c => format!("{c}x"),
        })
        .chain(["1. x".into(), "2) x".into(), "3.".into()]);
    for start in starts {
        let document = Document {
            schema: 1,
            title: "T".into(),
            fields: vec![],
            sections: vec![Section {
                heading: "Outcome".into(),
                body: Body::Prose(vec![
                    Block::Paragraph(vec![start.clone(), start.clone()]),
                    Block::List(vec![start.clone()]),
                ]),
            }],
        };
        let file = render(&document, &SHAPE);
        let file = file.unwrap_or_else(|error| panic!("{start:?}: {error:?}"));
        assert_eq!(outline(&file, &document), Ok(()), "{start:?}: {file}");
    }
}

/// A document of `lines` lines of prose `width` wide.
fn sized(lines: usize, width: usize) -> Document {
    let mut document = example();
    document.sections = vec![Section {
        heading: "Outcome".into(),
        body: Body::Prose(vec![Block::Paragraph(vec!["x".repeat(width); lines])]),
    }];
    document
}

#[test]
fn files_lines_and_line_counts_are_capped_at_their_bounds() {
    let file = render(&sized(3, 10), &SHAPE).unwrap();
    let lines = file.lines().count();
    let widest = file.lines().map(str::len).max().unwrap();
    #[rustfmt::skip]
    let cases = [
        (Limits { file_bytes: file.len(), ..LIMITS }, true),
        (Limits { file_bytes: file.len() - 1, ..LIMITS }, false),
        (Limits { lines, ..LIMITS }, true),
        (Limits { lines: lines - 1, ..LIMITS }, false),
        (Limits { line_bytes: widest, ..LIMITS }, true),
        (Limits { line_bytes: widest - 1, ..LIMITS }, false),
    ];
    for (limits, fits) in cases {
        let shape = Shape { limits, ..SHAPE };
        let rendered = render(&sized(3, 10), &shape);
        assert_eq!(rendered.is_ok(), fits, "{limits:?}: {rendered:?}");
    }
    // The text a document holds is counted before anything is rendered.
    let shape = Shape {
        limits: Limits {
            file_bytes: 99,
            ..LIMITS
        },
        ..SHAPE
    };
    assert_eq!(
        render(&sized(1, 100), &shape),
        Err(MarkdownError::TooLarge {
            bytes: text_bytes(&sized(1, 100)),
            most: 99
        })
    );
}

#[path = "markdown_hostile_tests.rs"]
mod hostile;
