//! The codec over well-formed documents: an exact file, round trips over
//! generated documents, prose that never makes a block, and the limits.
use super::*;

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
        body: BodyShape::Table(&["Item", "State"]),
    },
];

pub(super) const SHAPE: Shape<'static> = Shape {
    kind: "task",
    fields: &["Id", "Status", "Claim"],
    sections: SECTIONS,
    limits: LIMITS,
};

pub(super) fn example() -> Document {
    Document {
        schema: 1,
        title: "Boards S2a: quecto board CLI".into(),
        fields: vec![
            ("Id".into(), "`board-cli`".into()),
            ("Status".into(), "review".into()),
            (
                "Claim".into(),
                "Ada <ada@example.com> until 2026-10-11T14:00:00Z".replace(['<', '>'], ""),
            ),
        ],
        sections: vec![
            Section {
                heading: "Outcome".into(),
                body: Body::Prose(
                    "Agents read the board.\n- not a list\n\n# not a heading\nplain".into(),
                ),
            },
            Section {
                heading: "Items".into(),
                body: Body::Table(vec![
                    vec!["parse a|b".into(), "done".into()],
                    vec!["path C:\\x".into(), "open".into()],
                ]),
            },
        ],
    }
}

pub(super) const EXAMPLE: &str = "<!-- quecto-task schema=1 -->
# Boards S2a: quecto board CLI

| Field | Value |
|---|---|
| Id | `board-cli` |
| Status | review |
| Claim | Ada ada@example.com until 2026-10-11T14:00:00Z |

## Outcome
Agents read the board.
\\- not a list

\\# not a heading
plain

## Items
| Item | State |
|---|---|
| parse a\\|b | done |
| path C:\\\\x | open |
";

#[test]
fn a_document_renders_to_one_exact_file() {
    assert_eq!(render(&example(), &SHAPE).as_deref(), Ok(EXAMPLE));
}

/// A small, seeded generator: the same documents on every run.
struct Seeded(u64);

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

    /// Text of pieces that start blocks, escape, and stretch Unicode.
    fn text(&mut self, most: usize) -> String {
        #[rustfmt::skip]
        const PIECES: &[&str] = &[
            "a", "Z", "9", "1.", "#", "##", "|", ">", "=", "-", "---", "+", "*", "_", ":",
            "[x]", "<", "\\", "`code`", "~~~", "é", "Zoe\u{308}", "🦀", "👨\u{200d}👩\u{200d}👧",
            "€", "日本", " ", "  ", "!", "(", ")", "\"", "'", ".", ",", "{", "}", "&", "%", "$",
        ];
        let mut text = String::new();
        for _ in 0..1 + self.below(most) {
            text.push_str(PIECES[self.below(PIECES.len())]);
        }
        text
    }

    fn line(&mut self) -> String {
        self.text(8).trim().to_string()
    }

    fn document(&mut self) -> Document {
        let mut fields = vec![];
        for name in SHAPE.fields {
            if self.below(3) > 0 {
                fields.push(((*name).to_string(), self.line()));
            }
        }
        let mut sections = vec![];
        for section in SHAPE.sections {
            if self.below(3) == 0 {
                continue;
            }
            let body = match section.body {
                BodyShape::Prose => {
                    let lines: Vec<String> = (0..1 + self.below(5))
                        .map(|_| match self.below(4) {
                            0 => String::new(),
                            _ => self.line(),
                        })
                        .collect();
                    Body::Prose(lines.join("\n"))
                }
                BodyShape::Table(columns) => Body::Table(
                    (0..self.below(4))
                        .map(|_| columns.iter().map(|_| self.line()).collect())
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
            title: self.line(),
            fields,
            sections,
        }
    }
}

/// Every generated document renders or is refused with its reason named,
/// and a fair share of them render.
#[test]
fn every_generated_document_renders_or_is_refused_by_name() {
    let mut seeded = Seeded(0x9e37_79b9_7f4a_7c15);
    let mut taken = 0;
    for _ in 0..3_000 {
        let document = seeded.document();
        match render(&document, &SHAPE) {
            Ok(_) => taken += 1,
            Err(MarkdownError::NotCanonical) => panic!("{document:?}: refused unnamed"),
            Err(_) => {}
        }
    }
    assert!(taken > 1_000, "{taken} of 3000 taken");
}

/// Prose can never make a heading, a table row, a marker or any other
/// block: every line of it in a file that starts as a block might (by this
/// test's own list, CommonMark's and GFM's block starts) is escaped.
#[test]
fn prose_never_makes_a_heading_a_table_row_or_a_marker() {
    let starts_a_block = |line: &str| {
        line.chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || "#|>=-+*_:[<`~ ".contains(c))
    };
    let mut seeded = Seeded(7);
    let mut taken = 0;
    for _ in 0..2_000 {
        let prose = (0..3).map(|_| seeded.line()).collect::<Vec<_>>().join("\n");
        let document = Document {
            schema: 1,
            title: "T".into(),
            fields: vec![],
            sections: vec![Section {
                heading: "Outcome".into(),
                body: Body::Prose(prose),
            }],
        };
        let Ok(file) = render(&document, &SHAPE) else {
            continue;
        };
        taken += 1;
        let body = file.split_once("## Outcome\n").unwrap().1;
        for line in body.lines() {
            assert!(!starts_a_block(line), "{line:?} in {file}");
        }
    }
    assert!(taken > 500, "{taken} of 2000 taken");
}

/// A document of `line` lines of prose `width` wide.
fn sized(lines: usize, width: usize) -> Document {
    let mut document = example();
    let prose = vec!["x".repeat(width); lines].join("\n");
    document.sections = vec![Section {
        heading: "Outcome".into(),
        body: Body::Prose(prose),
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
}

#[path = "markdown_hostile_tests.rs"]
mod hostile;
