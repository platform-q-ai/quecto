//! The characters a file may hold, line by line: an allowlist. Matching
//! uses the `regex` crate alone, which never backtracks.
use super::MarkdownError;
use regex::Regex;
use std::sync::LazyLock;
use unicode_normalization::is_nfc;

/// Letters, marks, numbers, punctuation and symbols: the categories text
/// is made of. The only space is U+0020.
static SHOWN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\p{L}\p{M}\p{N}\p{P}\p{S}]$").expect("a valid class"));
static MARK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\p{M}$").expect("a valid class"));
/// Code points shown as nothing: refused but for an emoji's joiner and
/// presentation selector.
static IGNORABLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\p{Default_Ignorable_Code_Point}$").expect("a valid class"));
static EMOJI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\p{Extended_Pictographic}$").expect("a valid class"));

/// The most combining marks one base carries.
pub const MOST_MARKS: usize = 4;
const JOINER: char = '\u{200d}';

fn is(class: &Regex, c: char) -> bool {
    class.is_match(c.encode_utf8(&mut [0; 4]))
}

fn is_bidi(c: char) -> bool {
    matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

fn is_skin_tone(c: char) -> bool {
    ('\u{1f3fb}'..='\u{1f3ff}').contains(&c)
}

/// An emoji base and what may follow it before a joiner: nothing, one
/// skin-tone modifier, or one presentation selector.
fn ends_an_emoji(chars: &[char]) -> bool {
    match chars {
        [.., base] if is(&EMOJI, *base) => true,
        [.., base, follower] => {
            is(&EMOJI, *base) && (is_skin_tone(*follower) || *follower == '\u{fe0f}')
        }
        _ => false,
    }
}

/// Whether `chars[at]`, an ignorable code point, is one an emoji sequence
/// writes: a joiner between an emoji (as [`ends_an_emoji`]) and an emoji,
/// or one presentation selector straight after an emoji base.
fn in_emoji(chars: &[char], at: usize) -> bool {
    let before = &chars[..at];
    match chars[at] {
        JOINER => ends_an_emoji(before) && chars.get(at + 1).is_some_and(|c| is(&EMOJI, *c)),
        '\u{fe0e}' | '\u{fe0f}' => before.last().is_some_and(|c| is(&EMOJI, *c)),
        _ => false,
    }
}

/// The rules every line of a file keeps: allowed characters only, at most
/// [`MOST_MARKS`] combining marks on a base, Unicode NFC, and no trailing
/// space.
///
/// # Errors
/// The first rule the line breaks.
pub fn check_line(line: &str, at: usize) -> Result<(), MarkdownError> {
    let chars: Vec<char> = line.chars().collect();
    let mut marks = 0;
    for (index, &c) in chars.iter().enumerate() {
        marks = match is(&MARK, c) {
            true => marks + 1,
            false => 0,
        };
        match c {
            '\r' => return Err(MarkdownError::CarriageReturn { line: at }),
            ' ' => {}
            c if c.is_control() => return Err(MarkdownError::Control { line: at }),
            c if is_bidi(c) => return Err(MarkdownError::Bidi { line: at }),
            c if is(&IGNORABLE, c) && !in_emoji(&chars, index) => {
                return Err(MarkdownError::Invisible { line: at });
            }
            c if is(&IGNORABLE, c) => {}
            '\u{2800}' => return Err(MarkdownError::Invisible { line: at }),
            c if !is(&SHOWN, c) => return Err(MarkdownError::Invisible { line: at }),
            _ if marks > MOST_MARKS => return Err(MarkdownError::Marks { line: at }),
            _ => {}
        }
    }
    match (is_nfc(line), line.ends_with(' ')) {
        (true, false) => Ok(()),
        (false, _) => Err(MarkdownError::NotNfc { line: at }),
        (true, true) => Err(MarkdownError::TrailingSpace { line: at }),
    }
}
