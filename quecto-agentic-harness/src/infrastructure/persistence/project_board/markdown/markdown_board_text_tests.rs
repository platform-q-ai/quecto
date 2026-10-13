//! The board text normaliser against a reader (#2481 D4): normalising
//! text the domain accepts never changes what GitHub shows. comrak, with
//! the oracle's GitHub options and raw HTML rendered, renders the text
//! before and after, and the two agree exactly.
use super::*;
use crate::domain::project_board::value_objects::board_text;
use crate::domain::project_board::value_objects::schema_error::markdown;

/// What GitHub shows for `text`: its HTML as comrak renders it with the
/// oracle's GitHub options, raw HTML rendered as written (so an HTML
/// block that moves shows), comparing block structure, code and code-span
/// contents, and text.
fn shown(text: &str) -> String {
    let mut options = super::oracle::comrak_reader::comrak_options();
    options.render.unsafe_ = true;
    comrak::markdown_to_html(text, &options)
}

/// What the normaliser may change: spacing, line ends and blank lines;
/// every other character, in order, is the text's own.
fn content(text: &str) -> String {
    text.split([
        ' ', '\t', '\r', '\n', '\u{a0}', '\u{202f}', '\u{2007}', '\u{3000}',
    ])
    .collect()
}

/// What GitHub shows for `text`, a chat space read as the plain space it
/// looks like: the normaliser may make one plain, never change markup.
fn shown_plain(text: &str) -> String {
    shown(text).replace(['\u{a0}', '\u{202f}', '\u{2007}', '\u{3000}'], " ")
}

/// Pieces of text from chat, imports and editors: what D4 fixes (CR,
/// trailing spaces, decomposed letters, edge blank lines) among what it
/// must leave alone (fences of backticks or tildes and longer ones,
/// indents, tabs, no-break and wide spaces, selectors, hard breaks, list
/// continuations, code spans, quotes).
const PIECES: &[&str] = &[
    "a",
    "Z",
    "0",
    " ",
    "  ",
    "\t",
    "\u{a0}",
    "\u{3000}",
    "\r\n",
    "\r",
    "\n",
    "\n\n",
    "\n\n\n",
    "\u{fe0f}",
    "\u{1f600}",
    "e\u{301}",
    "\u{212b}",
    "- ",
    "  ",
    "    ",
    "# ",
    "> ",
    "1. ",
    "*",
    "_",
    "`",
    "`x`",
    "`\t`",
    "`a\u{a0}b`",
    "\n```\n",
    "\n```rust\n",
    "\n~~~\n",
    "\n````\n",
    "\n  ```\n",
    "line  \nnext",
    "- a\n  b",
    "- a\n\n  b",
    "\n\u{a0}\n",
    ".",
    "(",
    "\u{e9}",
    // What NFC or a stripped space could make into markup: escapes, math,
    // HTML and quotes, headings, rules, entities, and the characters NFC
    // turns into ASCII punctuation (U+0338 over `>`, U+1FEF, U+037E).
    "\\",
    "$",
    "<",
    ">",
    "#",
    "***",
    "\u{338}",
    "\u{1fef}",
    "\u{37e}",
    "&",
    "&lt",
    "a\\ \n",
    "`a\n# b`",
    "`a\n- b`",
    "`x\n> y`",
    "$x \n",
    "<https://a \n",
    "<!-- ",
    "\n<div>\n",
    "\n<pre>\n",
    "\u{212a}",
    "[a](/u \"t \n",
    "]: /u 't \n",
    // Round 4: titles and labels across lines, inline HTML across lines,
    // footnote definitions.
    "[l]: /u \n\"t \n",
    "[a](\n",
    "[a \nb](/u)",
    "<span title=\"x \n",
    "<?x \n",
    "<![CDATA[ x \n",
    "[^1]: ",
    "\n[^1]\n",
    // Round 3 (S7): chat spaces beside markup a plain space would make,
    // and between French words and punctuation.
    "x\u{202f}!",
    "a\u{a0}?",
    "\u{ab}\u{a0}",
    "\u{a0}#",
    "[\u{a0}]",
    "1.\u{a0}",
    "-\u{a0}",
    "\u{a0}*",
    "\u{3000}",
];

/// A small deterministic generator (an LCG), so a failure replays.
struct Seeded(u64);

impl Seeded {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as usize) % n
    }

    fn text(&mut self) -> String {
        (0..1 + self.below(24))
            .map(|_| PIECES[self.below(PIECES.len())])
            .collect()
    }
}

#[test]
fn normalising_never_changes_what_a_reader_shows() {
    let mut seeded = Seeded(2481);
    let (mut checked, mut held) = (0, 0);
    for _ in 0..60_000 {
        let raw = seeded.text();
        let normal = board_text::text(&raw);
        assert_eq!(board_text::text(&normal), normal, "{raw:?}");
        // Only spacing, line ends and blank lines change: no NFC, so text
        // written decomposed comes back as written.
        assert!(!normal.contains('\r'), "{raw:?}");
        assert_eq!(content(&normal), content(&raw), "{raw:?}");
        if markdown("description", &normal, 65_536).is_err() {
            continue;
        }
        checked += 1;
        assert_eq!(
            shown_plain(&raw),
            shown_plain(&normal),
            "{raw:?} → {normal:?}"
        );
        // What the codec holds, it holds as the normaliser wrote it.
        if let Ok(blocks) = blocks_of(&normal) {
            assert_eq!(text_of(&blocks), normal, "{raw:?}");
            held += 1;
        }
    }
    assert!(
        checked > 10_000 && held > 500,
        "{checked} checked, {held} held"
    );
}

/// Each class D4 fixes is refused by the codec as written and held once
/// normalised.
#[test]
fn each_class_d4_fixes_is_held_once_normalised() {
    for raw in [
        "Why.\r\nHow.\r\n",
        "a single \nspace",
        "trailing at the end  ",
        "abc\n",
        "\n\nedges\n\n",
    ] {
        assert!(blocks_of(raw).is_err(), "{raw:?} is refused as written");
        let normal = board_text::text(raw);
        let blocks = blocks_of(&normal).unwrap_or_else(|error| panic!("{raw:?}: {error:?}"));
        assert_eq!(text_of(&blocks), normal);
        assert_eq!(shown(raw), shown(&normal), "{raw:?}");
    }
}

/// What the normaliser leaves as written, the codec refuses with its line
/// (or holds as written): a hard break, a tab, an em space.
#[test]
fn what_d4_leaves_is_refused_by_the_codec_with_its_line() {
    for (raw, line) in [("line  \nnext", 1), ("a\tb", 1), ("a\n\nb\u{2003}c", 3)] {
        assert_eq!(board_text::text(raw), raw);
        let refused = blocks_of(raw).unwrap_err();
        assert!(
            format!("{refused}").contains(&format!("line {line}")),
            "{raw:?}: {refused}"
        );
    }
}
