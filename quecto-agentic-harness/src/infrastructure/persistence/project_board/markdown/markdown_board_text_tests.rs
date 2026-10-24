//! The board text normaliser against a reader (#2481 D4): normalising
//! text the domain accepts never changes what GitHub shows, past its chat
//! spaces (epic #2705 T1: made plain between words, which may change what
//! a reader sees in code and math, where an NBSP between words is never
//! meaningful; removed at a line's edge). comrak, with the oracle's GitHub
//! options and raw HTML rendered, renders text with chat spaces only
//! between words, those made plain, before and after, and the two agree
//! exactly. Chat spaces at a line's edge are checked against the codec.
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

const CHAT: [char; 4] = ['\u{a0}', '\u{202f}', '\u{2007}', '\u{3000}'];

/// Whether a line of `text` holds a chat space at its edge (in its leading
/// or trailing spacing, or as the whole line), which entry removes.
fn chat_at_an_edge(text: &str) -> bool {
    let spaced = |c: char| matches!(c, ' ' | '\t') || CHAT.contains(&c);
    text.replace("\r\n", "\n").split(['\n', '\r']).any(|line| {
        let start = &line[..line.len() - line.trim_start_matches(spaced).len()];
        let end = &line[line.trim_end_matches(spaced).len()..];
        start.contains(CHAT) || end.contains(CHAT)
    })
}

/// What the normaliser may change: spacing, line ends and blank lines;
/// every other character, in order, is the text's own.
fn content(text: &str) -> String {
    text.split([
        ' ', '\t', '\r', '\n', '\u{a0}', '\u{202f}', '\u{2007}', '\u{3000}',
    ])
    .collect()
}

/// What GitHub shows for `text` with each chat space made plain, as entry
/// makes each between words: the rest of the normaliser never changes
/// markup.
fn shown_plain(text: &str) -> String {
    shown(&text.replace(['\u{a0}', '\u{202f}', '\u{2007}', '\u{3000}'], " "))
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
    "\n```\n",
    "\n```rust\n",
    "\n~~~\n",
    "\n````\n",
    "\n  ```\n",
    "line  \nnext",
    "- a\n  b",
    "- a\n\n  b",
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
    // Addresses an autolink reads (S7 round 4), and markers beside which
    // a chat space is made plain too.
    "www.a.co",
    "https://a.co",
    "a@b.co",
    ">1.",
];

/// Pieces holding a chat space (no-break, narrow, figure, ideographic):
/// beside letters and French punctuation, and beside markup, where the
/// normaliser makes them plain all the same. Drawn one time in
/// [`CHAT_ONE_IN`].
const CHAT_PIECES: &[&str] = &[
    "\u{a0}",
    "\u{3000}",
    "`a\u{a0}b`",
    "\n\u{a0}\n",
    "x\u{202f}!",
    "a\u{a0}?",
    "\u{ab}\u{a0}",
    "\u{a0}#",
    "[\u{a0}]",
    "[x]\u{a0}",
    "1.\u{a0}",
    "-\u{a0}",
    "\u{a0}*",
    "a\u{2007}b",
    // Between words and punctuation, as French and Japanese keyboards
    // write them: the normaliser makes these plain.
    "b\u{a0}:",
    "\u{ab}\u{a0}c",
    "d\u{a0}\u{bb}",
    "1\u{a0}000",
    "e\u{3000}f",
    "g\u{202f};",
    "h\u{a0}%",
];

/// One draw in this many is a chat-space piece.
const CHAT_ONE_IN: usize = 30;

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
            .map(|_| match self.below(CHAT_ONE_IN) {
                0 => CHAT_PIECES[self.below(CHAT_PIECES.len())],
                _ => PIECES[self.below(PIECES.len())],
            })
            .collect()
    }
}

#[test]
fn normalising_never_changes_what_a_reader_shows() {
    let mut seeded = Seeded(2481);
    let (mut checked, mut held, mut plain) = (0, 0, 0);
    for _ in 0..20_000 {
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
        // A chat space between words was made plain by the normaliser; one
        // at a line's edge, removed, is the codec's to check (below).
        if !chat_at_an_edge(&raw) {
            plain += usize::from(raw.contains(CHAT));
            assert_eq!(
                shown_plain(&raw),
                shown_plain(&normal),
                "{raw:?} → {normal:?}"
            );
        }
        // What the codec holds, it holds as the normaliser wrote it.
        if let Ok(blocks) = blocks_of(&normal) {
            assert_eq!(text_of(&blocks), normal, "{raw:?}");
            held += 1;
        }
    }
    assert!(
        checked > 10_000 && held > 500 && plain > 200,
        "{checked} checked, {held} held, {plain} with chat spaces made plain"
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

/// Text the codec holds, chat spaces added only at its lines' edges or as
/// whole lines (as rich-text paste writes indents and empty paragraphs),
/// is held once normalised: entry removes each of them.
#[test]
fn chat_spaces_at_line_edges_never_stop_text_being_held() {
    const LINES: &[&str] = &[
        "Hello world",
        "Ça va ?",
        "`x` and y",
        "本文です。",
        "Steps:",
    ];
    let mut seeded = Seeded(2705);
    let edge = |seeded: &mut Seeded| -> String {
        (0..seeded.below(3))
            .map(|_| match seeded.below(4) {
                0 => ' ',
                _ => CHAT[seeded.below(CHAT.len())],
            })
            .collect()
    };
    for _ in 0..2_000 {
        let mut lines: Vec<String> = Vec::new();
        let (mut in_list, mut blocks) = (false, 1 + seeded.below(4));
        while blocks > 0 {
            blocks -= 1;
            // A list never follows a list: a reader would join them.
            let list = !in_list && seeded.below(2) == 0;
            in_list = list;
            for _ in 0..1 + seeded.below(3) {
                let word = LINES[seeded.below(LINES.len())];
                let marker = if list { "- " } else { "" };
                // A whole line of chat spaces, inside a block only.
                if seeded.below(4) == 0 && lines.last().is_some_and(|l| !l.is_empty()) {
                    lines.push(format!("{}{}", edge(&mut seeded), CHAT[0]));
                }
                // Chat spaces only at the edges: a plain one there is the
                // writer's own, which entry keeps.
                let lead = edge(&mut seeded).replace(' ', "");
                let tail = edge(&mut seeded).replace(' ', "");
                lines.push(format!("{lead}{marker}{word}{tail}"));
            }
            lines.push(String::new());
        }
        let raw = lines.join("\n");
        let normal = board_text::text(&raw);
        let blocks = blocks_of(&normal).unwrap_or_else(|error| panic!("{raw:?}: {error}"));
        assert_eq!(text_of(&blocks), normal, "{raw:?}");
    }
}

/// What the review found a wider normaliser changed in what GitHub shows
/// is left as written, but for its chat spaces, and the codec holds it as
/// written or refuses it with its line.
#[test]
fn what_a_reader_shows_differently_is_left_as_written() {
    for (raw, kept, held) in [
        ("\tcode", "\tcode", false),
        ("- a\n\n  b", "- a\n\n  b", false),
        ("`a\tb` `x\u{fe0f}`", "`a\tb` `x\u{fe0f}`", false),
        ("line  \nnext", "line  \nnext", false),
        ("A\u{fe0f} and a\tb", "A\u{fe0f} and a\tb", false),
        ("a\n\n\n\nb", "a\n\n\n\nb", false),
        ("   a", "   a", false),
        ("    a \nb", "    a \nb", false),
        ("a\n\u{a0}\nb", "a\n\nb", true),
        ("`a\u{a0}b` `a\u{3000}b`", "`a b` `a b`", true),
        ("\u{a0}- a", "- a", true),
    ] {
        let normal = board_text::text(raw);
        assert_eq!(normal, kept, "{raw:?}");
        match blocks_of(&normal) {
            Ok(blocks) => {
                assert!(held, "{raw:?} is held");
                assert_eq!(text_of(&blocks), normal, "{raw:?}");
            }
            Err(refused) => {
                assert!(!held, "{raw:?}: {refused}");
                assert!(format!("{refused}").contains("line "), "{raw:?}: {refused}");
            }
        }
    }
}
