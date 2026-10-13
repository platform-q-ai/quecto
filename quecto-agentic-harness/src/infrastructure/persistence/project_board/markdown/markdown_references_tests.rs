//! Link references in board markdown: a paragraph or list item that
//! would define one (`[label]:`, its label read raw, as CommonMark reads
//! it) is refused, written or read; a shortcut reference reads as text,
//! and a collapsed or full one is refused. Both oracle readers judge what
//! is held.
use super::*;

/// The example file with `from` replaced by `to` (once).
fn edited(from: &str, to: &str) -> Vec<u8> {
    assert!(EXAMPLE.contains(from), "{from:?}");
    EXAMPLE.replacen(from, to, 1).into_bytes()
}

/// The example document, its first section's prose `blocks`.
fn with_blocks(blocks: Vec<Block>) -> Document {
    let mut document = example();
    document.sections[0].body = Body::Prose(blocks);
    document
}

/// A paragraph that would define a link reference (its label on one line
/// or across several, its address bare or relative, with a title or
/// none) is refused, from a domain's text and when written: a definition
/// shows nothing, and gives every `[label]` on the page a hidden address.
/// Nested in a quote or a list item, the same text is no definition: a
/// quote's `>` is escaped, and an item opens with `[` only as a link.
#[test]
fn a_paragraph_that_defines_a_link_reference_is_refused() {
    for (text, line) in [
        ("See [docs].\n\n[docs]: //evil.example/x", 3),
        ("Hidden text below.\n\n[secret]: /x \"a hidden title\"", 3),
        ("[docs]: https://example.com", 1),
        ("[a\nb]: /x", 1),
        ("See [a b].\n\n[a\nb]:\n//evil.example/x", 3),
        ("[docs]:", 1),
        // A code span does not hide the label's end: CommonMark ends it at
        // the first `]`, so these define `` ` `` and show nothing.
        ("[`]: //evil.example/x \"`](https://example.com)\"", 1),
        (
            "Shown.\n\n[`]: //evil.example/x \"`](https://example.com)\"\nvisible tail",
            3,
        ),
        // A backslash-escaped `]` does not end it either.
        ("[a `\\]` b]: /x", 1),
    ] {
        assert_eq!(
            prose::blocks_of(text),
            Err(MarkdownError::ReferenceDefinition { line }),
            "{text:?}"
        );
        let paragraph: Vec<String> = text
            .rsplit("\n\n")
            .next()
            .unwrap()
            .split('\n')
            .map(str::to_string)
            .collect();
        let written = render(&with_blocks(vec![Block::Paragraph(paragraph)]), &SHAPE);
        assert_eq!(
            written,
            Err(MarkdownError::ReferenceDefinition { line: 11 }),
            "{text:?}"
        );
    }
    let quoted = with_blocks(prose::blocks_of("> [docs]: //evil.example/x").unwrap());
    let file = render(&quoted, &SHAPE).unwrap();
    assert!(file.contains("\\> [docs]: //evil.example/x"), "{file}");
    assert_eq!(outline(&file, &quoted), Ok(()), "{file}");
    assert!(
        prose::blocks_of("- [docs]: //evil.example/x").is_err(),
        "an item opens with `[` only as a link"
    );
    // What the writer is told: the owner-approved wording, byte for byte.
    assert_eq!(
        MarkdownError::ReferenceDefinition { line: 3 }.to_string(),
        "line 3: may not start a paragraph with [label]:, which markdown reads as a link \
         definition; reword it or start with other text"
    );
    // A paragraph may open with a link, or with a bracket that defines
    // nothing: one not closed by `]:`, or holding another `[`.
    for text in [
        "[docs](https://example.com) says so.",
        "[WIP] not done: soon",
        "[a [b]]: c",
        "[a [b]: c",
        "[`]`](https://example.com) a link whose text is code",
        "[docs] : spaced",
    ] {
        assert!(prose::blocks_of(text).is_ok(), "{text:?}");
    }
}

/// A file a writer edited by hand, its definition escaped as the renderer
/// escapes a line, is refused when read, as when written.
#[test]
fn a_definition_read_from_a_file_is_refused() {
    for to in [
        "\\[docs]: //evil.example/x\n",
        "\\[a\nb]: /x\n",
        "[`]: //evil.example/x \"`](https://example.com)\"\n",
    ] {
        let read = parse(
            &edited("Agents read the board, `Vec<T>` and all.\n", to),
            &SHAPE,
        );
        assert_eq!(
            read,
            Err(MarkdownError::ReferenceDefinition { line: 11 }),
            "{to:?}"
        );
    }
}

/// With no definition on the page, a shortcut reference resolves to
/// nothing: both readers show `[docs]` as text. A collapsed or full
/// reference is refused as a link of another form.
#[test]
fn a_shortcut_reference_reads_as_text_and_a_collapsed_one_is_refused() {
    let shortcut = with_blocks(prose::blocks_of("See [docs] and [a\nb].").unwrap());
    let file = render(&shortcut, &SHAPE).unwrap();
    assert_eq!(outline(&file, &shortcut), Ok(()), "{file}");
    let html = comrak::markdown_to_html(&file, &oracle::comrak_reader::comrak_options());
    assert!(!html.contains("<a "), "{html}");
    for text in ["See [docs][].", "See [docs][ref]."] {
        assert!(
            matches!(prose::blocks_of(text), Err(MarkdownError::Link { .. })),
            "{text:?}"
        );
    }
}

/// A list item opens as a definition too, its label's end hidden by a
/// code span or not: refused from a domain's text, when written, and when
/// read by hand. A bracket that defines nothing is held, both readers
/// showing it as text.
#[test]
fn a_list_item_that_defines_a_link_reference_is_refused() {
    let item = "[`]: //evil.example/x \"`](https://example.com)\"";
    assert_eq!(
        prose::blocks_of(&format!("- {item}")),
        Err(MarkdownError::ReferenceDefinition { line: 1 })
    );
    assert_eq!(
        render(&with_blocks(vec![Block::List(vec![item.into()])]), &SHAPE),
        Err(MarkdownError::ReferenceDefinition { line: 11 })
    );
    let read = parse(&edited("- read\n", &format!("- {item}\n")), &SHAPE);
    assert!(
        matches!(read, Err(MarkdownError::ReferenceDefinition { .. })),
        "{read:?}"
    );
    // Inside a paragraph no definition can start, and a code span binds
    // tighter than a link's brackets: the same line is the link it looks
    // like, written as is, and both readers outline it so.
    let inside = with_blocks(prose::blocks_of(&format!("Shown.\n{item}")).unwrap());
    let file = render(&inside, &SHAPE).unwrap();
    assert!(file.contains(&format!("\n{item}\n")), "{file}");
    assert_eq!(outline(&file, &inside), Ok(()), "{file}");
    for text in ["[a [b]: c", "Shown [a [b]: c."] {
        let held = with_blocks(prose::blocks_of(text).unwrap());
        let file = render(&held, &SHAPE).unwrap();
        assert_eq!(outline(&file, &held), Ok(()), "{file}");
    }
}
