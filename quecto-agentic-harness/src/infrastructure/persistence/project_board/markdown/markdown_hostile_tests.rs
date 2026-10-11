//! Hostile documents and files: each refused, with its reason named; and
//! the near misses a file must keep.
use super::*;

type Named = fn(&MarkdownError) -> bool;

/// The example file with `from` replaced by `to` (once).
fn edited(from: &str, to: &str) -> Vec<u8> {
    assert!(EXAMPLE.contains(from), "{from:?}");
    EXAMPLE.replacen(from, to, 1).into_bytes()
}

#[test]
fn every_hostile_file_is_refused_with_its_reason() {
    let mut huge = EXAMPLE.as_bytes().to_vec();
    huge.resize(LIMITS.file_bytes + 1, b'x');
    let not_utf8 = [
        &EXAMPLE.as_bytes()[..20],
        &[0xff],
        &EXAMPLE.as_bytes()[20..],
    ]
    .concat();
    let first = "Agents read the board, `Vec<T>` and all.";
    use MarkdownError as E;
    #[rustfmt::skip]
    let cases: Vec<(&str, Vec<u8>, Named)> = vec![
        ("raw HTML", edited("and all.", "and <b>all</b>."), |e| matches!(e, E::Html { line: 11 })),
        ("a comment", edited("and all.", "and <!-- x -->"), |e| matches!(e, E::Html { line: 11 })),
        ("a character reference", edited("and all.", "and &amp; all."), |e| matches!(e, E::CharacterReference { line: 11 })),
        ("dollar math", edited("and all.", "and $x$."), |e| matches!(e, E::Math { line: 11 })),
        ("an image", edited("and all.", "![i](https://example.com/i.png)"), |e| matches!(e, E::Image { line: 11 })),
        ("an http link", edited("https://example.com/docs", "http://example.com/docs"), |e| matches!(e, E::Link { line: 16 })),
        ("a stray escape", edited("and all.", "and all\\."), |e| matches!(e, E::Escape { line: 11 })),
        ("an open code span", edited("and all.", "and `all."), |e| matches!(e, E::UnbalancedCode { line: 11 })),
        ("a setext underline", edited("1\\. not a list\n", "1\\. not a list\n===\n"), |e| matches!(e, E::BlockLike { line: 14 })),
        ("a heading in a paragraph", edited("\\# not a heading", "# not a heading"), |e| matches!(e, E::BlockLike { line: 12 })),
        ("an ordered list in a paragraph", edited("1\\. not a list", "1. not a list"), |e| matches!(e, E::BlockLike { line: 13 })),
        ("a link reference definition", edited(first, "[a]: https://example.com"), |e| matches!(e, E::BlockLike { line: 11 })),
        ("a list in a paragraph", edited("\\# not a heading", "- an item"), |e| matches!(e, E::BlockLike { line: 12 })),
        ("an indented line", edited("\\# not a heading", "    code"), |e| matches!(e, E::Structure { line: 12, .. })),
        ("a lazy line after a list", edited("- read\n", "- read\nlazy\n"), |e| matches!(e, E::Structure { line: 16, .. })),
        ("a lazy line after a table", edited("| open |\n", "| open |\nlazy\n"), |e| matches!(e, E::Structure { line: 32, .. })),
        ("two blank lines", edited("1\\. not a list\n\n", "1\\. not a list\n\n\n"), |e| matches!(e, E::Structure { line: 15, .. })),
        ("an empty section", edited("## Acceptance\n- Every op answers one JSON line\n", "## Acceptance\n"), |e| matches!(e, E::EmptyProse { .. })),
        ("a fence left open", edited("}\n```\n", "}\n"), |e| matches!(e, E::Fence { line: 18, .. })),
        ("a fence's language in capitals", edited("```rust", "```Rust"), |e| matches!(e, E::Fence { line: 18, why: "a language from the list of plain highlighters" })),
        ("an indented close in code", edited("    let x = 1;", "   ```"), |e| matches!(e, E::Fence { line: 20, .. })),
        ("a longer close in code", edited("    let x = 1;", "````"), |e| matches!(e, E::Fence { line: 20, .. })),
        ("a bidi override", edited("and all.", "and a\u{202e}ll."), |e| matches!(e, E::Bidi { line: 11 })),
        ("a joiner between letters", edited("and all.", "and a\u{200d}ll."), |e| matches!(e, E::Invisible { line: 11 })),
        ("a zero-width space", edited("and all.", "and a\u{200b}ll."), |e| matches!(e, E::Invisible { line: 11 })),
        ("text not in NFC", edited("and all.", "and alle\u{301}."), |e| matches!(e, E::NotNfc { line: 11 })),
        ("a tab", edited("and all.", "and\tall."), |e| matches!(e, E::Control { line: 11 })),
        ("a CR", edited("and all.\n", "and all.\r\n"), |e| matches!(e, E::CarriageReturn { line: 11 })),
        ("a BOM", [&[0xef, 0xbb, 0xbf][..], EXAMPLE.as_bytes()].concat(), |e| matches!(e, E::ByteOrderMark)),
        ("a trailing space", edited("and all.", "and all. "), |e| matches!(e, E::TrailingSpace { line: 11 })),
        ("an oversize file", huge, |e| matches!(e, E::TooLarge { .. })),
        ("bytes that are not UTF-8", not_utf8, |e| matches!(e, E::NotUtf8)),
        ("no final newline", EXAMPLE.trim_end().as_bytes().to_vec(), |e| matches!(e, E::FinalNewline)),
        ("a blank last line", format!("{EXAMPLE}\n").into_bytes(), |e| matches!(e, E::FinalNewline)),
        ("another kind's marker", edited("quecto-task", "quecto-plan"), |e| matches!(e, E::Marker)),
        ("a schema with a leading zero", edited("schema=1", "schema=01"), |e| matches!(e, E::Marker)),
        ("no title", edited("# Boards", "Boards"), |e| matches!(e, E::Title)),
        ("a title closed by hashes", edited("CLI\n", "CLI #\n"), |e| matches!(e, E::Title)),
        ("a field no task has", edited("| Status |", "| Owner |"), |e| matches!(e, E::UnknownField { .. })),
        ("fields out of order", edited("| Id | board-cli |\n| Status | review |", "| Status | review |\n| Id | board-cli |"), |e| matches!(e, E::Structure { line: 7, .. })),
        ("a section no task has", edited("## Items", "## Notes"), |e| matches!(e, E::UnknownSection { .. })),
        ("a heading with no blank line", edited("\n## Acceptance", "## Acceptance"), |e| matches!(e, E::Structure { line: 23, .. })),
        ("a backslash opening a line", edited(first, "\\plain"), |e| matches!(e, E::BlockLike { line: 11 })),
        ("a link line escaped", edited(first, "\\[a](https://example.com)"), |e| matches!(e, E::NotCanonical)),
        ("a row of three cells", edited("| done |", "| done | x |"), |e| matches!(e, E::Cell { line: 30, .. })),
        ("a pipe that is no edge", edited("| review |", "| rev|iew |"), |e| matches!(e, E::Cell { line: 7, .. })),
        ("an escaped pipe in an id", edited("| review |", "| rev\\|iew |"), |e| matches!(e, E::Cell { line: 7, .. })),
        ("a cell padded twice", edited("| review |", "|  review |"), |e| matches!(e, E::Cell { line: 7, .. })),
        ("a pipe left unescaped in code", edited("`a\\|b`", "`a|b`"), |e| matches!(e, E::Cell { line: 30, .. })),
        ("a math fence", edited("```rust", "```math"), |e| matches!(e, E::Fence { line: 18, .. })),
        ("a mermaid fence", edited("```rust", "```mermaid"), |e| matches!(e, E::Fence { line: 18, .. })),
        ("a geojson fence", edited("```rust", "```geojson"), |e| matches!(e, E::Fence { line: 18, .. })),
        ("a topojson fence", edited("```rust", "```topojson"), |e| matches!(e, E::Fence { line: 18, .. })),
        ("an stl fence", edited("```rust", "```stl"), |e| matches!(e, E::Fence { line: 18, .. })),
        ("a link of a space", edited(first, "[ ](https://example.com)"), |e| matches!(e, E::Link { line: 11 })),
        ("a link of two spaces", edited(first, "[  ](https://example.com)"), |e| matches!(e, E::Link { line: 11 })),
        ("a link of a blank code span", edited(first, "[` `](https://example.com)"), |e| matches!(e, E::Link { line: 11 })),
        ("a link of a mark", edited(first, "[\u{301}](https://example.com)"), |e| matches!(e, E::Link { line: 11 })),
        ("a link of a blank note", edited(first, "[\u{1d159}](https://example.com)"), |e| matches!(e, E::Invisible { line: 11 })),
        ("a link with a space before", edited(first, "[ a](https://example.com)"), |e| matches!(e, E::Link { line: 11 })),
        ("a blank note", edited("and all.", "and a\u{1d159}ll."), |e| matches!(e, E::Invisible { line: 11 })),
        ("a Hangul filler", edited("and all.", "and a\u{3164}ll."), |e| matches!(e, E::Invisible { line: 11 })),
        ("a halfwidth Hangul filler", edited("and all.", "and a\u{ffa0}ll."), |e| matches!(e, E::Invisible { line: 11 })),
        ("a Hangul choseong filler", edited("and all.", "and a\u{115f}ll."), |e| matches!(e, E::Invisible { line: 11 })),
        ("a Hangul jungseong filler", edited("and all.", "and a\u{1160}ll."), |e| matches!(e, E::Invisible { line: 11 })),
        ("a selector after a digit", edited("and all.", "and 1\u{fe0f} all."), |e| matches!(e, E::Invisible { line: 11 })),
        ("a subdivision flag", edited("and all.", "and \u{1f3f4}\u{e0067}\u{e0062}\u{e007f}."), |e| matches!(e, E::Invisible { line: 11 })),
        ("more lines than a file holds", format!("{EXAMPLE}{}", "x\n".repeat(200_000)).into_bytes(), |e| matches!(e, E::TooManyLines { most: 8_192, .. })),
        ("an autolink hiding a span", edited(first, "a <1`@b.co> <!-- secret --> <img src=x> &amp; $x$ http://evil.example `"), |e| matches!(e, E::Html { line: 11 })),
        ("a destination hiding a span", edited(first, "see [a](https://b.co/`)<!-- secret --> <b>bold</b> &#65; `)"), |e| matches!(e, E::Link { line: 11 })),
        ("a span in a destination", edited(first, "see [a](https://x.co` `http://evil.example)"), |e| matches!(e, E::Link { line: 11 })),
        ("a line opening with a span in a destination", edited(first, "[a](https://x.co` `http://evil.example)"), |e| matches!(e, E::BlockLike { line: 11 })),
        ("a title hiding a span", edited("# Boards S2a: quecto board CLI", "# a <1`@b.co> <!-- x --> `"), |e| matches!(e, E::Html { line: 2 })),
        ("a list item hiding a span", edited("- Every op answers one JSON line", "- see [a](https://b.co/`)<b>x</b> `)"), |e| matches!(e, E::Link { line: 25 })),
        ("a text cell hiding a span", edited("| Ada (`ada@example.com`) until 2026-10-11T14:00:00Z |", "| [a](https://b.co/`)<b>x</b> `) |"), |e| matches!(e, E::Link { line: 8 })),
        ("an email autolink of a digit", edited(first, "mail <1@b.co> now"), |e| matches!(e, E::Html { line: 11 })),
        ("an email autolink of a plus", edited(first, "mail <+x@b.co>"), |e| matches!(e, E::Html { line: 11 })),
        ("an email autolink of an underscore", edited(first, "mail <_x@b.co>"), |e| matches!(e, E::Html { line: 11 })),
        ("an ftp address", edited(first, "see ftp://evil.example"), |e| matches!(e, E::Link { line: 11 })),
        ("any scheme", edited(first, "see gopher://evil.example"), |e| matches!(e, E::Link { line: 11 })),
        ("an http address in an id", edited("| board-cli |", "| http://evil.example |"), |e| matches!(e, E::Cell { line: 6, .. })),
        ("a www address in an id", edited("| board-cli |", "| www.evil.example |"), |e| matches!(e, E::Cell { line: 6, .. })),
        ("an email address in an id", edited("| board-cli |", "| a@b.co |"), |e| matches!(e, E::Cell { line: 6, .. })),
        ("a footnote", edited(first, "a note[^1]"), |e| matches!(e, E::Link { line: 11 })),
        ("a Khitan filler", edited(first, "a\u{16fe4}b"), |e| matches!(e, E::Invisible { line: 11 })),
        ("a line of a mark alone", edited(first, "\u{301}"), |e| matches!(e, E::Invisible { line: 11 })),
        ("a title of hashes alone", edited("# Boards S2a: quecto board CLI", "# ###"), |e| matches!(e, E::Title)),
        ("a link of emphasis around a blank span", edited(first, "see [*` `*](https://evil.example)"), |e| matches!(e, E::Link { line: 11 })),
        ("a link of emphasis around a mark", edited(first, "see [_\u{301}_](https://evil.example)"), |e| matches!(e, E::Link { line: 11 })),
        ("a title of emphasis around a blank span", edited("# Boards S2a: quecto board CLI", "# *` `*"), |e| matches!(e, E::Invisible { line: 2 })),
        ("a title of emphasis around a mark", edited("# Boards S2a: quecto board CLI", "# _\u{301}_"), |e| matches!(e, E::Invisible { line: 2 })),
        ("a text cell of emphasis around a mark", edited("| Ada (`ada@example.com`) until 2026-10-11T14:00:00Z |", "| _\u{301}_ |"), |e| matches!(e, E::Invisible { line: 8 })),
        ("a bare email", edited(first, "mail ada@example.com"), |e| matches!(e, E::Link { line: 11 })),
        ("a bare email in a text cell", edited("(`ada@example.com`)", "(ada@example.com)"), |e| matches!(e, E::Link { line: 8 })),
        ("a mailto address", edited(first, "see mailto:ada"), |e| matches!(e, E::Link { line: 11 })),
        ("a link that shows an address", edited(first, "see [www.evil.example](https://good.example)"), |e| matches!(e, E::Link { line: 11 })),
        ("www. after an underscore in an id", edited("| board-cli |", "| a_www.evil.example |"), |e| matches!(e, E::Cell { line: 6, .. })),
        ("an email of a dash in an id", edited("| board-cli |", "| a-@evil.example |"), |e| matches!(e, E::Cell { line: 6, .. })),
        ("emphasis by underscores in an id", edited("| board-cli |", "| _x_ |"), |e| matches!(e, E::Cell { line: 6, .. })),
        ("a link showing a www address in code", edited(first, "see [`www.evil.example`](https://good.example)"), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing an email in code", edited(first, "see [`ada@evil.example`](https://good.example)"), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing a scheme split by emphasis", edited(first, "see [http*s:*//evil.example/login](https://good.example)"), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing a www address split by strikethrough", edited(first, "see [ww~w~.evil.example](https://good.example)"), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing an email split by emphasis", edited(first, "see [ad*a*@evil.example](https://good.example)"), |e| matches!(e, E::Link { line: 11 })),
        ("an address with userinfo", edited("https://example.com/docs", "https://example.com@evil.example/docs"), |e| matches!(e, E::Link { line: 16 })),
        ("an escaped task box", edited("- Every op answers one JSON line", "- \\[x] done"), |e| matches!(e, E::Structure { line: 25, .. })),
        ("a task box", edited("- Every op answers one JSON line", "- [x] done"), |e| matches!(e, E::BlockLike { line: 25 })),
        ("punctuation opening a line, unescaped", edited(first, "(a note)"), |e| matches!(e, E::BlockLike { line: 11 })),
        ("a table's header renamed", edited("| Item | State |", "| Item | Status |"), |e| matches!(e, E::Structure { line: 28, .. })),
    ];
    for (name, bytes, named) in cases {
        let refused = parse(&bytes, &SHAPE);
        assert!(refused.as_ref().is_err_and(named), "{name}: {refused:?}");
    }
}

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
        ("a link of two spaces", render(&with_line("[  ](https://example.com)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
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
        ("an autolink hiding a span", render(&with_line("a <1`@b.co> <!-- secret --> <img src=x> &amp; $x$ http://evil.example `"), &SHAPE), |e| matches!(e, E::Html { line: 11 })),
        ("a destination hiding a span", render(&with_line("see [a](https://b.co/`)<!-- secret --> <b>bold</b> &#65; `)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a span in a destination", render(&with_line("[a](https://x.co` `http://evil.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a title hiding a span", with(|d| d.title = "a <1`@b.co> <!-- x --> `".into()), |e| matches!(e, E::Html { line: 2 })),
        ("a list item hiding a span", with(|d| d.sections[1].body = Body::Prose(vec![Block::List(vec!["see [a](https://b.co/`)<b>x</b> `)".into()])])), |e| matches!(e, E::Link { line: 25 })),
        ("a text cell hiding a span", render(&with_claim("[a](https://b.co/`)<b>x</b> `)"), &SHAPE), |e| matches!(e, E::Link { line: 8 })),
        ("an email autolink of a digit", render(&with_line("mail <1@b.co> now"), &SHAPE), |e| matches!(e, E::Html { line: 11 })),
        ("an email autolink of a plus", render(&with_line("<+x@b.co>"), &SHAPE), |e| matches!(e, E::Html { line: 11 })),
        ("an email autolink of an underscore", render(&with_line("<_x@b.co>"), &SHAPE), |e| matches!(e, E::Html { line: 11 })),
        ("an ftp address", render(&with_line("see ftp://evil.example"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("any scheme", render(&with_line("see gopher://evil.example"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("an ftp address in a text cell", render(&with_claim("ftp://evil.example"), &SHAPE), |e| matches!(e, E::Link { line: 8 })),
        ("an http address in an id", with(|d| d.fields[0].1 = "http://evil.example".into()), |e| matches!(e, E::Cell { line: 6, .. })),
        ("an ftp address in an id", with(|d| d.fields[0].1 = "ftp://evil.example".into()), |e| matches!(e, E::Cell { line: 6, .. })),
        ("a www address in an id", with(|d| d.fields[0].1 = "www.evil.example".into()), |e| matches!(e, E::Cell { line: 6, .. })),
        ("a www address after a separator in an id", with(|d| d.fields[0].1 = "see:www.evil.example".into()), |e| matches!(e, E::Cell { line: 6, .. })),
        ("an email address in an id", with(|d| d.fields[0].1 = "a@b.co".into()), |e| matches!(e, E::Cell { line: 6, .. })),
        ("a footnote", render(&with_line("a note[^1]"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a line longer than a file's lines, before anything renders", with(|d| {
            d.sections[0].body = Body::Prose(vec![Block::Paragraph(vec!["x".repeat(5_000)])]);
            d.sections[1].body = Body::Prose(vec![Block::Paragraph(vec!["<b>x</b>".into()])]);
        }), |e| matches!(e, E::LineTooLong { line: 11, most: 4_096 })),
        ("a Khitan filler", render(&with_line("a\u{16fe4}b"), &SHAPE), |e| matches!(e, E::Invisible { line: 11 })),
        ("a line of a mark alone", render(&with_line("\u{301}"), &SHAPE), |e| matches!(e, E::Invisible { line: 11 })),
        ("a line of a blank span alone", render(&with_line("` `"), &SHAPE), |e| matches!(e, E::Invisible { line: 11 })),
        ("a title of a mark alone", with(|d| d.title = "\u{301}".into()), |e| matches!(e, E::Invisible { line: 2 })),
        ("a text cell of a mark alone", render(&with_claim("\u{301}"), &SHAPE), |e| matches!(e, E::Invisible { line: 8 })),
        ("a title of hashes alone", with(|d| d.title = "###".into()), |e| matches!(e, E::Title)),
        ("a title closed by two hashes", with(|d| d.title = "a ##".into()), |e| matches!(e, E::Title)),
        ("a link of emphasis around a blank span", render(&with_line("[*` `*](https://evil.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link of emphasis around a mark", render(&with_line("[_\u{301}_](https://evil.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link of strikethrough around a mark", render(&with_line("[~~\u{301}~~](https://evil.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link of strong emphasis around a mark", render(&with_line("[__\u{301}__](https://evil.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a title of emphasis around a blank span", with(|d| d.title = "*` `*".into()), |e| matches!(e, E::Invisible { line: 2 })),
        ("a title of emphasis around a mark", with(|d| d.title = "_\u{301}_".into()), |e| matches!(e, E::Invisible { line: 2 })),
        ("a text cell of emphasis around a mark", render(&with_claim("_\u{301}_"), &SHAPE), |e| matches!(e, E::Invisible { line: 8 })),
        ("a line of markers alone", render(&with_line("** __ ~~"), &SHAPE), |e| matches!(e, E::Invisible { line: 11 })),
        ("a bare email", render(&with_line("mail ada@example.com"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a bare email in a text cell", render(&with_claim("Ada (ada@example.com)"), &SHAPE), |e| matches!(e, E::Link { line: 8 })),
        ("a mailto address", render(&with_line("see mailto:ada"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("an xmpp address", render(&with_line("see XMPP:ada"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link that shows an address", render(&with_line("[www.evil.example](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link that shows a scheme", render(&with_line("[https://evil.example](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link that shows an email", render(&with_line("[ada@evil.example](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("www. after an underscore in an id", with(|d| d.fields[0].1 = "a_www.evil.example".into()), |e| matches!(e, E::Cell { line: 6, .. })),
        ("WWW. in capitals in an id", with(|d| d.fields[0].1 = "x.WWW.evil.example".into()), |e| matches!(e, E::Cell { line: 6, .. })),
        ("an email of a dash in an id", with(|d| d.fields[0].1 = "a-@evil.example".into()), |e| matches!(e, E::Cell { line: 6, .. })),
        ("an email of a dot in an id", with(|d| d.fields[0].1 = "a.@evil.example".into()), |e| matches!(e, E::Cell { line: 6, .. })),
        ("an email of a plus in an id", with(|d| d.fields[0].1 = "a+@evil.example".into()), |e| matches!(e, E::Cell { line: 6, .. })),
        ("an email of a dash alone in an id", with(|d| d.fields[0].1 = "-@evil.example".into()), |e| matches!(e, E::Cell { line: 6, .. })),
        ("an email to a dot in an id", with(|d| d.fields[0].1 = "a@.evil.example".into()), |e| matches!(e, E::Cell { line: 6, .. })),
        ("an email to a dash in an id", with(|d| d.fields[0].1 = "a@-evil.example".into()), |e| matches!(e, E::Cell { line: 6, .. })),
        ("emphasis by underscores in an id", with(|d| d.fields[0].1 = "_x_".into()), |e| matches!(e, E::Cell { line: 6, .. })),
        ("a link showing a www address in code", render(&with_line("[`www.evil.example`](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing a scheme in code", render(&with_line("[`https://evil.example`](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing an email in code", render(&with_line("[`ada@evil.example`](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing a www address split by emphasis", render(&with_line("[ww*w*.evil.example](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing a scheme split by emphasis", render(&with_line("[http*s:*//evil.example/login](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing an email split by emphasis", render(&with_line("[ad*a*@evil.example](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing a www address split by strikethrough", render(&with_line("[ww~w~.evil.example](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing a www address split by code", render(&with_line("[w`w`w.evil.example](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing an email split by underscores", render(&with_line("[ad_a_@evil.example](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("an address with userinfo", render(&with_line("[a](https://good.example@evil.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("an address with userinfo and a path", render(&with_line("[a](https://good.example@evil.example/login)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a checked task", with(|d| d.sections[1].body = Body::Prose(vec![Block::List(vec!["[x] done".into()])])), |e| matches!(e, E::Structure { line: 25, .. })),
        ("an open task", with(|d| d.sections[1].body = Body::Prose(vec![Block::List(vec!["[ ] to do".into()])])), |e| matches!(e, E::Structure { line: 25, .. })),
        ("a link showing a www address in padded code", render(&with_line("[www` . `evil.example](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing an email in padded code", render(&with_line("[ada` @ `evil.example](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing a scheme in padded code", render(&with_line("[https:` // `evil.example](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing mailto in padded code", render(&with_line("[mailto` : `ada](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing an email after an underscore in code", render(&with_line("[_`@`evil.example](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link showing an email after an underscore in strikethrough", render(&with_line("[_~@evil.example~](https://good.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a scheme split across a link's start", render(&with_line("see https:/[/bank.example/login](https://evil.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a www address split across a link's start", render(&with_line("see ww[w.bank.example](https://evil.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("an email split across a link's start", render(&with_line("see ada[@bank.example](https://evil.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a scheme split across code and a link", render(&with_line("see `https:/`[/bank.example/login](https://evil.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a scheme split across a link's end", render(&with_line("[https:](https://evil.example)//bank.example"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a www address split across a link and code", render(&with_line("[www](https://evil.example)`.bank.example`"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a www address with a mark inside, in a link", render(&with_line("[www\u{332}.bank.example](https://evil.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a www address with a mark inside, across a link", render(&with_line("ww[w](https://evil.example)\u{332}.bank.example"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("an email with a mark inside, in a link", render(&with_line("[ada\u{332}@bank.example](https://evil.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        // Refused by design: a full stop or a literal marker beside a link
        // makes the run of text around it read as an address.
        ("a link a full stop makes www.", render(&with_line("the [www](https://w3.org)."), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a link a literal marker joins to an email", render(&with_line("[ada](https://x.example)*@b.example"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        // Refused by design: an underscore around a link is read as kept, as
        // a reader may show it.
        ("an underscore around a link to a handle", render(&with_line("see _[@ada](https://github.com/ada)_"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("two underscores around a link to a handle", render(&with_line("see __[@ada](https://github.com/ada)__"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("runs of unequal length around a link", render(&with_line("see ___[@b.example](https://x.example)__"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("two then one underscore around a link", render(&with_line("see __[@evil.example](https://x.example)_"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("underscores around code in a link", render(&with_line("see [__`@`evil.example_](https://x.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("an opener left over before a link", render(&with_line("see _a!__[@evil.example](https://x.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("emphasis cancelling an underscore in a link", render(&with_line("see [*a _`@`evil.example* c_](https://x.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("the rule of three in a link", render(&with_line("see [_`@`evil.example __y_ z_](https://x.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("strong emphasis opening a link beside an email", render(&with_line("see ada`@`b.example[** x](https://x.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("strong emphasis closing a link beside an email", render(&with_line("see [x **](https://x.example)ada`@`b.example"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
        ("a tilde opening a link beside an email", render(&with_line("see ada`@`b.example[~ x](https://x.example)"), &SHAPE), |e| matches!(e, E::Link { line: 11 })),
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
        "a < b, 1 < 2 and a trailing <",
        "times 12:00:00 and :smile: stay as written",
        "[a `]` b](https://x.example) opens the line as a link",
        "an email in code `ada@example.com` and a handle @ada",
        "a profile [here](https://example.com/@ada) and [paypal.com](https://example.com)",
        "README.md and C:\\x",
        "snake_case and a_b_c",
        "```a``` opens the line as code, not a fence",
    ] {
        let document = with_line(line);
        let file = render(&document, &SHAPE).unwrap_or_else(|error| panic!("{line:?}: {error:?}"));
        assert_eq!(outline(&file, &document), Ok(()), "{line:?}");
        assert_eq!(parse(file.as_bytes(), &SHAPE), Ok(document), "{line:?}");
    }
    let document = with_claim("`C:\\x` and `a|b`");
    let file = render(&document, &SHAPE).unwrap();
    assert!(file.contains("| Claim | `C:\\x` and `a\\|b` |"), "{file}");
    assert_eq!(outline(&file, &document), Ok(()));
    assert_eq!(parse(file.as_bytes(), &SHAPE), Ok(document));
}

/// The text around each link is read once: many links on one long line
/// are checked in time linear in the line.
#[test]
fn many_links_on_one_line_are_checked_in_linear_time() {
    let line = "[a](https://x.example) "
        .repeat(20_000)
        .trim_end()
        .to_string();
    let started = std::time::Instant::now();
    assert_eq!(inline::check_inline(&line, 1), Ok(()));
    let took = started.elapsed();
    assert!(
        took < std::time::Duration::from_secs(2),
        "{took:?} for 20,000 links"
    );
}

/// A list item may open with a link, which is no task box.
#[test]
fn a_list_item_may_open_with_a_link() {
    let mut document = example();
    document.sections[1].body = Body::Prose(vec![Block::List(vec![
        "[the docs](https://example.com/docs) first".into(),
        "[x](https://example.com/x)".into(),
    ])]);
    let file = render(&document, &SHAPE).unwrap();
    assert!(
        file.contains("\n- [the docs](https://example.com/docs) first\n"),
        "{file}"
    );
    assert_eq!(outline(&file, &document), Ok(()));
}

/// Every fence language on the list is kept, read as code by a reader.
#[test]
fn every_listed_fence_language_is_kept() {
    for &language in prose::LANGUAGES {
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

/// Ids, times, issue numbers and `path:line` locations are ASCII cells a
/// file keeps, as written.
#[test]
fn ascii_cells_keep_ids_times_and_locations() {
    for id in [
        "2026-10-11T12:00:00Z",
        "src/a.rs:88",
        "#2494",
        "board-cli",
        "v1.2.3+build",
        "in_progress",
        "feat/my_repo",
        "@scope/pkg",
    ] {
        let mut document = example();
        document.fields[0].1 = id.into();
        let file = render(&document, &SHAPE).unwrap_or_else(|error| panic!("{id}: {error:?}"));
        assert!(file.contains(&format!("| Id | {id} |")), "{id}");
        assert_eq!(outline(&file, &document), Ok(()), "{id}");
        assert_eq!(parse(file.as_bytes(), &SHAPE), Ok(document), "{id}");
    }
}

/// A title may end in a `#` that is part of a word; only a closing run
/// (a space then hashes) or hashes alone are refused.
#[test]
fn a_title_may_end_in_a_hash_of_its_own() {
    for title in ["Port to C#", "C#", "F# and C##"] {
        let mut document = example();
        document.title = title.into();
        let file = render(&document, &SHAPE).unwrap_or_else(|error| panic!("{title}: {error:?}"));
        assert_eq!(outline(&file, &document), Ok(()), "{title}");
        assert_eq!(parse(file.as_bytes(), &SHAPE), Ok(document), "{title}");
    }
}

/// Every refusal says what was refused, and where when it knows; it is a
/// standard error.
#[test]
fn every_refusal_says_what_and_where() {
    use MarkdownError as E;
    let refusals = [
        (E::Html { line: 11 }, "line 11"),
        (E::Link { line: 3 }, "line 3"),
        (
            E::Fence {
                line: 18,
                why: "a language from the list of plain highlighters",
            },
            "plain highlighters",
        ),
        (E::TooManyLines { lines: 9, most: 8 }, "9"),
        (E::Title, "title"),
        (E::NotCanonical, "canonical"),
        (
            E::NotReadBack {
                why: Some(Box::new(E::Title)),
            },
            "title",
        ),
    ];
    for (refusal, says) in refusals {
        let error: &dyn std::error::Error = &refusal;
        assert!(error.to_string().contains(says), "{refusal:?}: {error}");
    }
}

/// A file that does not read back as the document it was written from is
/// refused, in release builds too, so it is never stored.
#[test]
fn a_file_that_reads_back_otherwise_is_refused() {
    let mut other = example();
    other.title = "Another".into();
    assert_eq!(
        read_back(EXAMPLE.to_string(), &example(), &SHAPE).as_deref(),
        Ok(EXAMPLE)
    );
    // Read back as another document: no reason but that.
    assert_eq!(
        read_back(EXAMPLE.to_string(), &other, &SHAPE),
        Err(MarkdownError::NotReadBack { why: None })
    );
    // Not read back at all: the reason it was refused is kept.
    assert_eq!(
        read_back("not a board file\n".to_string(), &other, &SHAPE),
        Err(MarkdownError::NotReadBack {
            why: Some(Box::new(MarkdownError::Marker))
        })
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
        let refused = parse(EXAMPLE.as_bytes(), &shape);
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
