//! What the artifact codecs need of the markdown (#2481, M2b-1): words
//! with underscores in an ASCII cell, code cells, and a domain's markdown
//! text mapped losslessly to blocks.
use super::*;

const CELLS: &[Column<'static>] = &[
    Column {
        name: "Status",
        cell: CellKind::Ascii,
    },
    Column {
        name: "Branch",
        cell: CellKind::Code,
    },
];

const NOTE: &[SectionShape<'static>] = &[SectionShape {
    heading: "Note",
    body: BodyShape::Prose,
}];

const CELL_SHAPE: Shape<'static> = Shape {
    kind: "toy",
    fields: CELLS,
    sections: NOTE,
    limits: LIMITS,
};

fn with_cells(status: &str, branch: &str) -> Document {
    Document {
        schema: 1,
        title: "Cells".into(),
        fields: vec![
            ("Status".into(), status.into()),
            ("Branch".into(), branch.into()),
        ],
        sections: vec![],
    }
}

/// The value cell of the field table's row `row` (from 0), as a reader
/// reads it: its text runs and code spans, each tagged.
fn read_cell(file: &str, row: usize) -> Vec<String> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let (mut rows, mut cell, mut seen) = (0usize, 0usize, vec![]);
    let mut in_body = false;
    for event in Parser::new_ext(file, Options::all()) {
        match event {
            Event::End(TagEnd::TableHead) => in_body = true,
            Event::Start(Tag::TableRow) => cell = 0,
            Event::End(TagEnd::TableRow) => rows += 1,
            Event::End(TagEnd::TableCell) => cell += 1,
            Event::Text(text) if in_body && rows == row && cell == 1 => {
                seen.push(format!("text:{text}"));
            }
            Event::Code(text) if in_body && rows == row && cell == 1 => {
                seen.push(format!("code:{text}"));
            }
            _ => {}
        }
    }
    seen
}

#[test]
fn an_ascii_cell_holds_words_with_underscores() {
    for word in ["in_progress", "wont_fix", "changes_requested", "gpt_5"] {
        let document = with_cells(word, "main");
        let file = render(&document, &CELL_SHAPE).unwrap();
        assert!(file.contains(&format!("| Status | {word} |\n")), "{file}");
        assert_eq!(parse(file.as_bytes(), &CELL_SHAPE), Ok(document));
        assert_eq!(read_cell(&file, 0), [format!("text:{word}")]);
    }
}

/// `_` only between two letters or digits, where a reader never takes it
/// for emphasis: the cell shows as the text it holds.
#[test]
fn an_ascii_cell_refuses_an_underscore_a_reader_could_take_for_emphasis() {
    for word in ["my_repo", "a_b_c", "x1_2y", "feat/my_repo"] {
        let document = with_cells(word, "main");
        let file = render(&document, &CELL_SHAPE).unwrap();
        assert_eq!(parse(file.as_bytes(), &CELL_SHAPE), Ok(document));
        assert_eq!(read_cell(&file, 0), [format!("text:{word}")]);
    }
    for refused in [
        "_x", "x_", "_x_", "__init__", "a__b", "_", "x._y", "x_.y", "a/_b",
    ] {
        let rendered = render(&with_cells(refused, "main"), &CELL_SHAPE);
        assert!(
            matches!(rendered, Err(MarkdownError::Cell { .. })),
            "{refused:?}: {rendered:?}"
        );
        // Nor is it read from a file.
        let file = render(&with_cells("open", "main"), &CELL_SHAPE).unwrap();
        let edited = file.replacen("| Status | open |", &format!("| Status | {refused} |"), 1);
        assert!(parse(edited.as_bytes(), &CELL_SHAPE).is_err(), "{edited}");
    }
}

#[test]
fn a_code_cell_renders_in_backticks_and_reads_back() {
    for branch in [
        "feat/a<b>",
        "[::1]:5000/x",
        "a*b*_c_",
        "x",
        // A pipe is written `\|`, as a text cell writes it; a reader shows
        // it, and a backslash, as they are.
        "a|b",
        "a\\b",
        "a\\|b",
        "|",
        "\\",
        "x|",
    ] {
        let document = with_cells("open", branch);
        let file = render(&document, &CELL_SHAPE).unwrap();
        let written = branch.replace('|', "\\|");
        assert!(
            file.contains(&format!("| Branch | `{written}` |\n")),
            "{file}"
        );
        assert_eq!(parse(file.as_bytes(), &CELL_SHAPE), Ok(document.clone()));
        assert_eq!(read_cell(&file, 1), [format!("code:{branch}")]);
        assert_eq!(super::outline(&file, &document), Ok(()), "{file}");
    }
    for refused in ["a`b", "`", "a b", "", "é", "a\tb"] {
        let rendered = render(&with_cells("open", refused), &CELL_SHAPE);
        assert!(
            matches!(rendered, Err(MarkdownError::Cell { .. })),
            "{refused:?}: {rendered:?}"
        );
    }
    // A code cell is read only in its backticks.
    let file = render(&with_cells("open", "main"), &CELL_SHAPE).unwrap();
    for (from, to) in [
        ("`main`", "main"),
        ("`main`", "`main"),
        ("`main`", "``main``"),
    ] {
        let edited = file.replacen(from, to, 1);
        assert!(parse(edited.as_bytes(), &CELL_SHAPE).is_err(), "{edited}");
    }
}

#[test]
fn the_schema_is_read_from_the_marker_line_only() {
    let file = render(&with_cells("open", "main"), &CELL_SHAPE).unwrap();
    assert_eq!(schema_of(file.as_bytes(), "toy"), Some(1));
    assert_eq!(
        schema_of(b"<!-- quecto-toy schema=7 -->\n\xff\xfe", "toy"),
        Some(7)
    );
    for unread in [
        &b"<!-- quecto-toy schema=7 -->"[..],
        b"<!-- quecto-task schema=7 -->\n",
        b"<!-- quecto-toy schema=07 -->\n",
        b"<!-- quecto-toy schema= -->\n",
        b"\n<!-- quecto-toy schema=7 -->\n",
        b"<!-- quecto-toy schema=99999999999 -->\n",
    ] {
        assert_eq!(schema_of(unread, "toy"), None, "{unread:?}");
    }
}

/// `text`'s blocks, which give `text` again, and read back from a file.
fn round_trip(text: &str) -> Vec<Block> {
    let blocks = blocks_of(text).unwrap_or_else(|error| panic!("{text:?}: {error:?}"));
    assert_eq!(text_of(&blocks), text);
    let document = Document {
        schema: 1,
        title: "Prose".into(),
        fields: vec![],
        sections: vec![Section {
            heading: "Note".into(),
            body: Body::Prose(blocks.clone()),
        }],
    };
    let file = render(&document, &CELL_SHAPE).unwrap();
    assert_eq!(parse(file.as_bytes(), &CELL_SHAPE), Ok(document));
    blocks
}

#[test]
fn prose_text_round_trips_through_blocks() {
    assert_eq!(blocks_of(""), Ok(vec![]));
    assert_eq!(text_of(&[]), "");
    assert_eq!(round_trip("# x"), [Block::Paragraph(vec!["# x".into()])]);
    assert_eq!(
        round_trip("1. x\n2) y"),
        [Block::Paragraph(vec!["1. x".into(), "2) y".into()])]
    );
    assert_eq!(
        round_trip("```rust\nfn a() {\n\n    1\n}\n```"),
        [Block::Code {
            language: Some("rust".into()),
            lines: vec!["fn a() {".into(), "".into(), "    1".into(), "}".into()],
        }]
    );
    assert_eq!(
        round_trip("- a\n- b\n\nafter\nmore"),
        [
            Block::List(vec!["a".into(), "b".into()]),
            Block::Paragraph(vec!["after".into(), "more".into()]),
        ]
    );
    assert_eq!(
        round_trip("a\n- b\n\n```\n\n```\n\n- c"),
        [
            Block::Paragraph(vec!["a".into(), "- b".into()]),
            Block::Code {
                language: None,
                lines: vec!["".into()],
            },
            Block::List(vec!["c".into()]),
        ]
    );
}

#[test]
fn prose_text_blocks_cannot_hold_is_refused() {
    for refused in [
        "- a\n\n- b",
        "a\n",
        "a\n\n",
        "\na",
        "a\n\n\nb",
        " a",
        "a\n b",
        "a\tb",
        "a\u{a0}b",
        "```\nopen",
        "```\nx\n```\nafter",
        "```math\nx\n```",
        "- ",
    ] {
        let blocks = blocks_of(refused);
        assert!(blocks.is_err(), "{refused:?}: {blocks:?}");
    }
}

/// The longest marker `schema_of` reads: a kind of 24 letters and ten
/// digits; one byte more is no marker.
#[test]
fn the_longest_marker_is_read_and_one_byte_more_is_not() {
    let kind = "abcdefghijklmnopqrstuvwx";
    assert_eq!(kind.len(), 24);
    let longest = format!("<!-- quecto-{kind} schema=4294967296 -->\n");
    assert_eq!(schema_of(longest.as_bytes(), kind), Some(4_294_967_296));
    let longer = format!("<!-- quecto-{kind} schema=42949672960 -->\n");
    assert_eq!(schema_of(longer.as_bytes(), kind), None);
    // A kind one letter past the 24 a shape allows, with ten digits: 59
    // bytes, past the longest marker.
    let wide = "abcdefghijklmnopqrstuvwxy";
    let past = format!("<!-- quecto-{wide} schema=4294967296 -->\n");
    assert_eq!(past.len() - 1, 59);
    assert_eq!(schema_of(past.as_bytes(), wide), None);
}
