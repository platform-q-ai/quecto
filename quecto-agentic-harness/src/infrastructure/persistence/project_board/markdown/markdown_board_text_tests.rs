//! The board text normaliser against a reader (#2481 D4): normalising
//! text the domain accepts never changes what GitHub shows. comrak, with
//! the oracle's GitHub options, renders the text before and after: the
//! two agree once both are NFC (canonically equal text shows the same).
use super::*;
use crate::domain::project_board::value_objects::board_text;
use crate::domain::project_board::value_objects::schema_error::markdown;
use unicode_normalization::UnicodeNormalization;

/// What GitHub shows for `text`: its HTML as comrak renders it with the
/// oracle's GitHub options (block structure, code and code-span contents,
/// text), made NFC.
fn shown(text: &str) -> String {
    comrak::markdown_to_html(text, &super::oracle::comrak_reader::comrak_options())
        .nfc()
        .collect()
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
    let mut checked = 0;
    for _ in 0..20_000 {
        let raw = seeded.text();
        let normal = board_text::text(&raw);
        assert_eq!(board_text::text(&normal), normal, "{raw:?}");
        assert!(
            !normal.contains('\r') && normal.nfc().eq(normal.chars()),
            "{raw:?}"
        );
        if markdown("description", &normal, 65_536).is_err() {
            continue;
        }
        checked += 1;
        assert_eq!(shown(&raw), shown(&normal), "{raw:?} → {normal:?}");
    }
    assert!(checked > 10_000, "{checked} checked");
}

/// Each class D4 fixes is refused by the codec as written and held once
/// normalised.
#[test]
fn each_class_d4_fixes_is_held_once_normalised() {
    for raw in [
        "Why.\r\nHow.\r\n",
        "a single \nspace",
        "trailing at the end  ",
        "Cafe\u{301}",
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
/// (or holds as written): a hard break, a tab, a no-break space.
#[test]
fn what_d4_leaves_is_refused_by_the_codec_with_its_line() {
    for (raw, line) in [("line  \nnext", 1), ("a\tb", 1), ("a\n\nb\u{a0}c", 3)] {
        assert_eq!(board_text::text(raw), raw);
        let refused = blocks_of(raw).unwrap_err();
        assert!(
            format!("{refused}").contains(&format!("line {line}")),
            "{raw:?}: {refused}"
        );
    }
}
