//! The board text normaliser against the codec (#2481 D4): text the
//! domain accepts, once normalised, is held by the codec's prose, or is
//! refused for what it means (its structure or inline markup), never for
//! what the normaliser exists to fix.
use super::*;
use crate::domain::project_board::value_objects::board_text;
use crate::domain::project_board::value_objects::schema_error::markdown;

/// What the codec refuses for what text means, not how it is spelt: the
/// refusals the normaliser deliberately leaves to the codec.
fn for_meaning(error: &MarkdownError) -> bool {
    matches!(
        error,
        MarkdownError::Structure { .. }
            | MarkdownError::UnbalancedCode { .. }
            | MarkdownError::BlockLike { .. }
            | MarkdownError::Html { .. }
            | MarkdownError::Link { .. }
            | MarkdownError::Escape { .. }
            | MarkdownError::Math { .. }
            | MarkdownError::CharacterReference { .. }
            | MarkdownError::Image { .. }
            | MarkdownError::Marks { .. }
    )
}

/// Pieces of text from chat, imports and editors: what the domain lets
/// through and the codec refuses as spelt (CR, tabs, no-break and wide
/// spaces, lone selectors, decomposed letters, edge and doubled blank
/// lines), among ordinary text and markdown. No fence: inside one the
/// normaliser changes only what shows as nothing, by design.
const PIECES: &[&str] = &[
    "a",
    "Z",
    "0",
    " ",
    "  ",
    "\t",
    "\u{a0}",
    "\u{2003}",
    "\u{3000}",
    "\r\n",
    "\r",
    "\n",
    "\n\n",
    "\n\n\n",
    "\u{fe0f}",
    "\u{fe0e}",
    "\u{1f600}",
    "\u{2764}\u{fe0f}",
    "1\u{fe0f}\u{20e3}",
    "e\u{301}",
    "\u{212b}",
    "- ",
    "# ",
    "*",
    "_",
    "`",
    "`x`",
    ".",
    ",",
    "(",
    ")",
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
fn normalised_text_the_domain_accepts_is_held_or_refused_for_its_meaning() {
    let mut seeded = Seeded(2481);
    let (mut held, mut checked) = (0, 0);
    for _ in 0..20_000 {
        let raw = seeded.text();
        let normal = board_text::text(&raw);
        assert_eq!(board_text::text(&normal), normal, "{raw:?}");
        if markdown("description", &normal, 65_536).is_err() {
            continue;
        }
        checked += 1;
        match blocks_of(&normal) {
            Ok(blocks) => {
                assert_eq!(text_of(&blocks), normal, "{raw:?}");
                held += 1;
            }
            Err(error) => assert!(for_meaning(&error), "{raw:?} → {normal:?}: {error:?}"),
        }
    }
    assert!(
        checked > 10_000 && held > 3_000,
        "{checked} checked, {held} held"
    );
}

/// Each class of text the domain accepts and the codec refuses as spelt
/// is held once normalised.
#[test]
fn each_class_the_codec_refuses_as_spelt_is_held_once_normalised() {
    for raw in [
        "Why.\r\nHow.\r\n",
        "trailing  \nspaces\t",
        "no\u{a0}break",
        "a\ttab",
        "A\u{fe0f} after a letter",
        "Cafe\u{301}",
        "abc\n",
        "\n\nedges\n\n",
        "one\n\n\n\nblank",
    ] {
        assert!(blocks_of(raw).is_err(), "{raw:?} is refused as spelt");
        let normal = board_text::text(raw);
        let blocks = blocks_of(&normal).unwrap_or_else(|error| panic!("{raw:?}: {error:?}"));
        assert_eq!(text_of(&blocks), normal);
    }
}
