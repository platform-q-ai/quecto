//! The codec over well-formed documents: an exact file, generated
//! documents read back by a CommonMark/GFM reader (the oracle) and by
//! `parse`, edits of a file refused or read as the oracle reads them,
//! block starts kept as text, and the limits.
use super::*;
use oracle::outline;
use pulldown_cmark::{Event, Options, Parser, Tag};

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
                "Ada (`ada@example.com`) until 2026-10-11T14:00:00Z".into(),
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
| Claim | Ada (`ada@example.com`) until 2026-10-11T14:00:00Z |

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

/// The oracle tells apart what the codec must never write: each file
/// below is the example with one block changed as a reader would see it.
#[test]
fn the_oracle_refuses_a_file_outlined_otherwise() {
    #[rustfmt::skip]
    let cases = [
        ("a loose list", "- read\n- write", "- read\n\n- write"),
        ("a block in an item", "- read\n", "- read\n\n  more\n"),
        ("a nested list", "- read\n", "- read\n  - deeper\n"),
        ("a paragraph line lost", "\\# not a heading\n", ""),
        ("a paragraph line gained", "1\\. not a list\n", "1\\. not a list\nmore\n"),
        ("a code line gained", "}\n```", "}\nx\n```"),
        ("a table row gained", "| path `C:\\x` | open |\n", "| path `C:\\x` | open |\n| x | y |\n"),
        ("a header of another width", "| Item | State |\n|---|---|", "| Item | State | More |\n|---|---|---|"),
        ("an ordered list", "- Every op", "1. Every op"),
    ];
    for (name, from, to) in cases {
        assert!(EXAMPLE.contains(from), "{name}");
        let file = EXAMPLE.replacen(from, to, 1);
        assert!(outline(&file, &example()).is_err(), "{name}: {file}");
    }
}

#[test]
fn a_document_renders_to_one_exact_file_a_reader_outlines_as_it() {
    assert_eq!(render(&example(), &SHAPE).as_deref(), Ok(EXAMPLE));
    assert_eq!(outline(EXAMPLE, &example()), Ok(()));
    assert_eq!(parse(EXAMPLE.as_bytes(), &SHAPE), Ok(example()));
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
            "[a link](https://example.com/x)", "[a `]` b](https://x.example)", "` . `", "` @ `",
            "__", "___", "`@`", "**", "[** x](https://x.example)", "[x **](https://x.example)",
            "[~ x](https://x.example)", "[_y_](https://x.example)", "x\u{301}\u{302}",
        ];
        #[rustfmt::skip]
        const HOSTILE: &[&str] = &[
            "\u{1}", "\u{202e}", "\u{200b}", "\u{fe0f}", "e\u{301}", "&amp;", "&#65;", "$x$",
            "![i](https://example.com/i.png)", "<b>", "http://x", "www.x", "\u{2800}", "\u{3164}",
            "a\u{fe0f}\u{200d}🦀", "\u{a0}", "[r][ref]", "[[w]]", "[^1]", "ftp://x", "<1@b.co>", "ada@example.com",
            "_www.x", "*` `*", "_\u{301}_", "mailto:x", "[www.x](https://example.com)",
            "[www` . `x.y](https://example.com)", "[_`@`x.y](https://example.com)", "x\u{301}\u{302}\u{303}\u{304}\u{305}",
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
        const ASCII: &[&str] = &["a", "z", "0", "9", ".", "a_b", ":", "/", "#", "+", "-"];
        let mut cell: String = (0..1 + self.below(8)).map(|_| self.pick(ASCII)).collect();
        if self.below(20) == 0 {
            cell.insert(0, '@');
        }
        if self.below(40) == 0 {
            cell.push_str(&self.pick(&[" ", "é", "|", "@x", "_", "www.x"]));
        }
        cell
    }

    /// A code cell: the ASCII cell's characters and more punctuation, now
    /// and then one a code cell refuses.
    fn code(&mut self) -> String {
        const CODE: &[&str] = &["a", "Z", "0", "<", ">", "[", "]", "*", "$", "&", "~", "!"];
        let mut cell = self.ascii();
        cell.extend((0..self.below(4)).map(|_| self.pick(CODE)));
        if self.below(40) == 0 {
            cell.push_str(&self.pick(&["`", "\\", "|"]));
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
            CellKind::Code => self.code(),
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
/// outlines as the document and `parse` reads back as it; a fair share
/// are taken, and a fair share refused.
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
                assert_eq!(parse(file.as_bytes(), &SHAPE), Ok(document), "{file}");
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
    let starts = (b'!'..=b'~')
        .map(char::from)
        .filter(char::is_ascii_punctuation)
        .map(|c| match c {
            '`' => "`a` x".to_string(),
            '$' => "$ x".to_string(),
            '<' => "< x".to_string(),
            c => format!("{c}x"),
        })
        .chain(["1. x".into(), "2) x".into(), "3.".into()])
        .chain(["```a``` x".into(), "``~~~`` x".into()]);
    for start in starts {
        let mut blocks = vec![Block::Paragraph(vec![start.clone(), start.clone()])];
        // An item opens with `[` only as a link.
        if !start.starts_with('[') {
            blocks.push(Block::List(vec![start.clone()]));
        }
        let document = Document {
            schema: 1,
            title: "T".into(),
            fields: vec![],
            sections: vec![Section {
                heading: "Outcome".into(),
                body: Body::Prose(blocks),
            }],
        };
        let file = render(&document, &SHAPE);
        let file = file.unwrap_or_else(|error| panic!("{start:?}: {error:?}"));
        assert_eq!(outline(&file, &document), Ok(()), "{start:?}: {file}");
        // Every ASCII punctuation a line opens with is escaped, but the
        // backtick, whose span the line closes.
        let punctuation = start.starts_with(|c: char| c.is_ascii_punctuation() && c != '`');
        if punctuation {
            assert!(
                file.contains(&format!("\n\\{start}\n")),
                "{start:?}: {file}"
            );
        }
        assert_eq!(parse(file.as_bytes(), &SHAPE), Ok(document), "{start:?}");
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
        let parsed = parse(file.as_bytes(), &shape);
        assert_eq!(parsed.is_ok(), fits, "{limits:?}: {parsed:?}");
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

/// Every edit of the example (a character taken out, or a piece put in at
/// any point) is refused, or read as a document a CommonMark/GFM reader
/// outlines the edited file as, and that renders to it.
#[test]
fn every_edit_of_a_file_is_refused_or_read_as_a_reader_reads_it() {
    #[rustfmt::skip]
    const PIECES: &[&str] = &[
        "\n", "\n\n", "\\", "`", "```", "   ```\n", " ", "-", "- ", "#", "|", "1. ", "[[", "<b>", "\u{200d}",
    ];
    let starts: Vec<usize> = (0..=EXAMPLE.len())
        .filter(|at| EXAMPLE.is_char_boundary(*at))
        .collect();
    let (mut read, mut refused) = (0, 0);
    for (index, &at) in starts.iter().enumerate() {
        let without = starts
            .get(index + 1)
            .map(|&next| format!("{}{}", &EXAMPLE[..at], &EXAMPLE[next..]));
        let with = PIECES
            .iter()
            .map(|piece| format!("{}{piece}{}", &EXAMPLE[..at], &EXAMPLE[at..]));
        for file in without.into_iter().chain(with) {
            match parse(file.as_bytes(), &SHAPE) {
                Ok(document) => {
                    read += 1;
                    assert_eq!(outline(&file, &document), Ok(()), "{file}");
                    assert_eq!(render(&document, &SHAPE).as_deref(), Ok(file.as_str()));
                }
                // A refusal that names a line names one of the file's.
                Err(error) => {
                    refused += 1;
                    let said = error.to_string();
                    let line = said
                        .strip_prefix("line ")
                        .and_then(|rest| rest.split_once(':'))
                        .map(|(line, _)| line.parse::<usize>().unwrap());
                    let lines = file.lines().count();
                    assert!(
                        line.is_none_or(|line| (1..=lines).contains(&line)),
                        "{said}: {file}"
                    );
                }
            }
        }
    }
    assert!(
        read > 500 && refused > 2_000,
        "{read} edits read, {refused} refused"
    );
}

#[path = "markdown_oracle_tests.rs"]
mod oracle;

#[path = "markdown_hostile_tests.rs"]
mod hostile;

#[path = "markdown_cells_tests.rs"]
mod cells;
