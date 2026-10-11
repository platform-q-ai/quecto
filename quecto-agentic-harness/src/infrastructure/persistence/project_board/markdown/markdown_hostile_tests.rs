//! Hostile documents: each refused when written, with its reason named;
//! and the near misses a file must keep.
use super::*;

type Named = fn(&MarkdownError) -> bool;

/// The example with its first paragraph's first line `text`.
fn with_line(text: &str) -> Document {
    let mut document = example();
    if let Body::Prose(blocks) = &mut document.sections[0].body {
        blocks[0] = Block::Paragraph(vec![text.to_string()]);
    }
    document
}

/// Prose of one empty code block in `language`.
fn code_in(language: &str) -> Body {
    Body::Prose(vec![Block::Code {
        language: Some(language.into()),
        lines: vec![],
    }])
}

/// The example with its Claim (a text cell) `text`.
fn with_claim(text: &str) -> Document {
    let mut document = example();
    document.fields[2].1 = text.into();
    document
}

#[test]
fn hostile_documents_are_refused_when_written() {
    let with = |change: fn(&mut Document)| {
        let mut document = example();
        change(&mut document);
        render(&document, &SHAPE)
    };
    use MarkdownError as E;
    #[rustfmt::skip]
    let cases: Vec<(&str, Result<String, E>, Named)> = vec![
        ("raw HTML", render(&with_line("a <b>x</b>"), &SHAPE), |e| matches!(e, E::Html { line: 11 })),
        ("a comment", render(&with_line("a <!-- x -->"), &SHAPE), |e| matches!(e, E::Html { .. })),
        ("a named reference", render(&with_line("a &amp; b"), &SHAPE), |e| matches!(e, E::CharacterReference { .. })),
        ("a numeric reference", render(&with_line("a &#x41; b"), &SHAPE), |e| matches!(e, E::CharacterReference { .. })),
        ("inline math", render(&with_line("a $x$ b"), &SHAPE), |e| matches!(e, E::Math { .. })),
        ("a price", render(&with_line("costs $5"), &SHAPE), |e| matches!(e, E::Math { .. })),
        ("an image", render(&with_line("![i](https://example.com/i.png)"), &SHAPE), |e| matches!(e, E::Image { .. })),
        ("an http link", render(&with_line("[a](http://example.com)"), &SHAPE), |e| matches!(e, E::Link { .. })),
        ("an empty link", render(&with_line("[](https://example.com)"), &SHAPE), |e| matches!(e, E::Link { .. })),
        ("a titled link", render(&with_line("[a](https://example.com \"t\")"), &SHAPE), |e| matches!(e, E::Link { .. })),
        ("a reference link", render(&with_line("[a][r]"), &SHAPE), |e| matches!(e, E::Link { .. })),
        ("a wikilink", render(&with_line("a [[x]] b"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a bare address", render(&with_line("see https://example.com"), &SHAPE), |e| matches!(e, E::Link { .. })),
        ("an autolink", render(&with_line("<https://example.com>"), &SHAPE), |e| matches!(e, E::Html { .. })),
        ("an escape", render(&with_line("a\\*b"), &SHAPE), |e| matches!(e, E::Escape { .. })),
        ("an escaped backtick", render(&with_line("a \\`b` <T>"), &SHAPE), |e| matches!(e, E::Escape { .. })),
        ("a hard break", render(&with_line("a\\"), &SHAPE), |e| matches!(e, E::Escape { .. })),
        ("an open code span", render(&with_line("a `b"), &SHAPE), |e| matches!(e, E::UnbalancedCode { .. })),
        ("a leading space", render(&with_line(" indented"), &SHAPE), |e| matches!(e, E::Structure { .. })),
        ("a bidi override", render(&with_line("pla\u{202e}in"), &SHAPE), |e| matches!(e, E::Bidi { .. })),
        ("a zero-width space", render(&with_line("pl\u{200b}ain"), &SHAPE), |e| matches!(e, E::Invisible { .. })),
        ("a no-break space", render(&with_line("pl\u{a0}ain"), &SHAPE), |e| matches!(e, E::Invisible { .. })),
        ("a Hangul filler", render(&with_line("pl\u{3164}ain"), &SHAPE), |e| matches!(e, E::Invisible { .. })),
        ("a braille blank", render(&with_line("pl\u{2800}ain"), &SHAPE), |e| matches!(e, E::Invisible { .. })),
        ("a selector after a letter", render(&with_line("a\u{fe0f}"), &SHAPE), |e| matches!(e, E::Invisible { .. })),
        ("a joiner after a selector after a letter", render(&with_line("a\u{fe0f}\u{200d}🦀"), &SHAPE), |e| matches!(e, E::Invisible { .. })),
        ("five marks on a base", render(&with_line("x\u{301}\u{302}\u{303}\u{304}\u{305}"), &SHAPE), |e| matches!(e, E::Marks { .. })),
        ("text not in NFC", render(&with_line("Zoe\u{308}"), &SHAPE), |e| matches!(e, E::NotNfc { .. })),
        ("a tab", render(&with_line("pl\tain"), &SHAPE), |e| matches!(e, E::Control { .. })),
        ("an id with a space", with(|d| d.fields[0].1 = "board cli".into()), |e| matches!(e, E::Cell { .. })),
        ("an id with a pipe", with(|d| d.fields[0].1 = "a|b".into()), |e| matches!(e, E::Cell { .. })),
        ("an id not in ASCII", with(|d| d.fields[0].1 = "bóard".into()), |e| matches!(e, E::Cell { .. })),
        ("HTML in a cell", render(&with_claim("<script>"), &SHAPE), |e| matches!(e, E::Html { line: 8 })),
        ("a newline in a cell", render(&with_claim("a\nb"), &SHAPE), |e| matches!(e, E::Cell { .. })),
        ("an empty cell", render(&with_claim(""), &SHAPE), |e| matches!(e, E::Cell { .. })),
        ("a padded cell", render(&with_claim(" a"), &SHAPE), |e| matches!(e, E::Cell { .. })),
        ("HTML in a title", with(|d| d.title = "a <script>".into()), |e| matches!(e, E::Html { line: 2 })),
        ("a two-line title", with(|d| d.title = "a\nb".into()), |e| matches!(e, E::Title)),
        ("a title closed by hashes", with(|d| d.title = "a #".into()), |e| matches!(e, E::Title)),
        ("empty prose", with(|d| d.sections[0].body = Body::Prose(vec![])), |e| matches!(e, E::EmptyProse { .. })),
        ("two lists in a row", with(|d| d.sections[0].body = Body::Prose(vec![Block::List(vec!["a".into()]), Block::List(vec!["b".into()])])), |e| matches!(e, E::Structure { .. })),
        ("an empty list", with(|d| d.sections[0].body = Body::Prose(vec![Block::List(vec![])])), |e| matches!(e, E::Structure { .. })),
        ("a fence's own close in code", with(|d| d.sections[0].body = Body::Prose(vec![Block::Code { language: None, lines: vec!["```".into()] }])), |e| matches!(e, E::Fence { .. })),
        ("an indented close in code", with(|d| d.sections[0].body = Body::Prose(vec![Block::Code { language: None, lines: vec!["   ```".into()] }])), |e| matches!(e, E::Fence { .. })),
        ("a longer indented close in code", with(|d| d.sections[0].body = Body::Prose(vec![Block::Code { language: None, lines: vec!["x".into(), " ````".into()] }])), |e| matches!(e, E::Fence { line: 13, .. })),
        ("a math fence", with(|d| d.sections[0].body = code_in("math")), |e| matches!(e, E::Fence { line: 11, .. })),
        ("a mermaid fence", with(|d| d.sections[0].body = code_in("mermaid")), |e| matches!(e, E::Fence { .. })),
        ("a geojson fence", with(|d| d.sections[0].body = code_in("geojson")), |e| matches!(e, E::Fence { .. })),
        ("a topojson fence", with(|d| d.sections[0].body = code_in("topojson")), |e| matches!(e, E::Fence { .. })),
        ("an stl fence", with(|d| d.sections[0].body = code_in("stl")), |e| matches!(e, E::Fence { .. })),
        ("a link of a space", render(&with_line("[ ](https://example.com)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link of a blank code span", render(&with_line("[` `](https://example.com)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link of a mark", render(&with_line("[\u{301}](https://example.com)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link of a blank note", render(&with_line("[\u{1d159}](https://example.com)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link with a space before", render(&with_line("[ a](https://example.com)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link with a space after", render(&with_line("[a ](https://example.com)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a blank note", render(&with_line("a\u{1d159}b"), &SHAPE), |e| matches!(e, E::Invisible { line: 11 })),
        ("a halfwidth Hangul filler", render(&with_line("a\u{ffa0}b"), &SHAPE), |e| matches!(e, E::Invisible { line: 11 })),
        ("a Hangul choseong filler", render(&with_line("a\u{115f}b"), &SHAPE), |e| matches!(e, E::Invisible { line: 11 })),
        ("a Hangul jungseong filler", render(&with_line("a\u{1160}b"), &SHAPE), |e| matches!(e, E::Invisible { line: 11 })),
        ("a selector after a digit", render(&with_line("1\u{fe0f} x"), &SHAPE), |e| matches!(e, E::Invisible { line: 11 })),
        ("a selector after a digit, then no keycap", render(&with_line("1\u{fe0f}x\u{20e3}"), &SHAPE), |e| matches!(e, E::Invisible { line: 11 })),
        ("a subdivision flag", render(&with_line("\u{1f3f4}\u{e0067}\u{e0062}\u{e0073}\u{e0063}\u{e0074}\u{e007f}"), &SHAPE), |e| matches!(e, E::Invisible { line: 11 })),
        ("a fence language with a space", with(|d| d.sections[0].body = Body::Prose(vec![Block::Code { language: Some("a b".into()), lines: vec![] }])), |e| matches!(e, E::Fence { .. })),
        ("a row of one cell", with(|d| d.sections[2].body = Body::Table(vec![vec!["x".into()]])), |e| matches!(e, E::Cell { .. })),
        ("a field no task has", with(|d| d.fields[0].0 = "Owner".into()), |e| matches!(e, E::UnknownField { .. })),
        ("fields out of order", with(|d| d.fields.swap(0, 1)), |e| matches!(e, E::Structure { .. })),
        ("a prose section as a table", with(|d| d.sections[0].body = Body::Table(vec![])), |e| matches!(e, E::UnknownSection { .. })),
    ];
    for (name, outcome, named) in cases {
        assert!(outcome.as_ref().is_err_and(named), "{name}: {outcome:?}");
    }
}

/// Near misses a file must keep, each read by the oracle as it is.
#[test]
fn near_misses_are_kept() {
    for line in [
        "a coder 👩🏽‍💻 at work",
        "a family 👨‍👩‍👧",
        "a heart ❤️",
        "generic `Vec<T>` and `a <b> c`",
        "see [the docs](https://example.com/a/b?c=d)",
        "a $ sign alone",
        "a < b and c > d",
        "C:\\x and a\\b",
        "x\u{301}\u{302}\u{303}\u{304}",
        "keycaps 1\u{fe0f}\u{20e3} #\u{fe0f}\u{20e3} *\u{fe0f}\u{20e3} and 2\u{20e3}",
        "[`code`](https://example.com) and [a b](https://example.com)",
    ] {
        let document = with_line(line);
        let file = render(&document, &SHAPE).unwrap_or_else(|error| panic!("{line:?}: {error:?}"));
        assert_eq!(outline(&file, &document), Ok(()), "{line:?}");
    }
    let document = with_claim("`C:\\x` and `a|b`");
    let file = render(&document, &SHAPE).unwrap();
    assert!(file.contains("| Claim | `C:\\x` and `a\\|b` |"), "{file}");
    assert_eq!(outline(&file, &document), Ok(()));
}

/// Every fence language on the list is kept, read as code by a reader.
#[test]
fn every_listed_fence_language_is_kept() {
    for language in [
        "text", "sh", "bash", "console", "rust", "toml", "json", "yaml", "diff", "python", "js",
        "ts", "kotlin", "java", "go", "sql", "html", "css", "markdown", "c", "cpp",
    ] {
        let mut document = example();
        document.sections[0].body = Body::Prose(vec![Block::Code {
            language: Some(language.into()),
            lines: vec!["x".into()],
        }]);
        let file =
            render(&document, &SHAPE).unwrap_or_else(|error| panic!("{language}: {error:?}"));
        assert_eq!(outline(&file, &document), Ok(()), "{language}");
    }
}

/// The lines a document makes are counted before anything is rendered: a
/// code block of more empty lines than a file may hold is refused as such,
/// not after rendering what comes before it.
#[test]
fn lines_are_counted_before_anything_is_rendered() {
    let mut document = example();
    document.sections[0].body = Body::Prose(vec![Block::Code {
        language: None,
        lines: vec![String::new(); 200_000],
    }]);
    document.sections[1].body = Body::Prose(vec![Block::Paragraph(vec!["<b>x</b>".into()])]);
    let refused = render(&document, &SHAPE);
    assert!(
        matches!(
            refused,
            Err(MarkdownError::TooManyLines { most: 8_192, .. })
        ),
        "{refused:?}"
    );
}

/// A shape is checked before anything is rendered.
#[test]
fn a_shape_no_file_can_be_is_refused() {
    use MarkdownError as E;
    let empty: &[Column<'static>] = &[];
    let twice = &[FIELDS[0], FIELDS[0]];
    #[rustfmt::skip]
    let cases = [
        Shape { kind: "Task", ..SHAPE },
        Shape { kind: "", ..SHAPE },
        Shape { fields: twice, ..SHAPE },
        Shape { fields: &[Column { name: "lower", cell: CellKind::Text }], ..SHAPE },
        Shape { sections: &[SectionShape { heading: "A|B", body: BodyShape::Prose }], ..SHAPE },
        Shape { sections: &[SectionShape { heading: "Rows", body: BodyShape::Table(empty) }], ..SHAPE },
    ];
    for shape in cases {
        let refused = render(&example(), &shape);
        assert!(
            matches!(refused, Err(E::Shape(_))),
            "{shape:?}: {refused:?}"
        );
    }
}

/// A line that opens with a link keeps it a link: a reference definition
/// it cannot be, so it is not escaped.
#[test]
fn a_line_that_opens_with_a_link_stays_a_link() {
    let document = with_line("[the docs](https://example.com/docs) come first");
    let file = render(&document, &SHAPE).unwrap();
    assert!(
        file.contains("\n[the docs](https://example.com/docs) come first\n"),
        "{file}"
    );
    let links = Parser::new_ext(&file, Options::all())
        .filter(|event| matches!(event, Event::Start(Tag::Link { .. })))
        .count();
    assert_eq!(links, 2, "{file}");
    assert_eq!(outline(&file, &document), Ok(()));
}
