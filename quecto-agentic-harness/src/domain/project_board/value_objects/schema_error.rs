//! Why a task was refused, and the bounded-text checks every field shares.
use regex::Regex;
use std::fmt;
use std::sync::LazyLock;

/// A task that breaks the schema: the field and what it must be. `field`
/// is the full path to the value, keys and list indexes joined by `/`
/// (`depends_on/3`, `claim/holder/email`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    pub field: String,
    pub problem: String,
}

impl SchemaError {
    pub fn new(field: impl Into<String>, problem: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            problem: problem.into(),
        }
    }

    /// The same refusal, for the value at `field`.
    pub fn at(self, field: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            ..self
        }
    }
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.field, self.problem)
    }
}

impl std::error::Error for SchemaError {}

/// A visible character: Unicode L*, M*, N*, P* or S*, except the default
/// ignorables (variation selectors, the combining grapheme joiner, the
/// Mongolian selectors, the Hangul fillers and the like, which show
/// nothing) and the braille blank U+2800.
const SHOWN: &str = r"[[\p{L}\p{M}\p{N}\p{P}\p{S}]--[\p{Default_Ignorable_Code_Point}\x{2800}]]";
/// One presentation selector (U+FE0E text style, U+FE0F emoji style),
/// allowed only straight after a visible character.
const PRESENTATION: &str = r"[\x{FE0E}\x{FE0F}]";
/// A subdivision flag: a black flag U+1F3F4, a two-letter country and one
/// to four letters or digits of subdivision in lowercase tag characters,
/// then the cancel tag U+E007F (England's `gbeng`). Tags spell nothing
/// else, so they cannot carry hidden text.
const FLAG: &str =
    r"\x{1F3F4}[\x{E0061}-\x{E007A}]{2}[\x{E0030}-\x{E0039}\x{E0061}-\x{E007A}]{1,4}\x{E007F}";
/// ZWNJ U+200C and ZWJ U+200D, each only ever between two glyphs (👩‍💻,
/// 🏳️‍🌈, Persian ZWNJ). Every other format character (bidi marks such as
/// LRM, RLM and ALM, the soft hyphen, zero-width spaces, the BOM), every
/// control, private-use and unassigned character is refused.
const JOINER: &str = r"[\x{200C}\x{200D}]";

/// Glyphs (a visible character with its optional presentation selector,
/// or a flag), possibly joined, and the given separators.
fn text(separators: &str) -> Regex {
    let glyph = format!("(?:{SHOWN}{PRESENTATION}?|{FLAG})");
    Regex::new(&format!("^(?:{glyph}(?:{JOINER}{glyph})*|{separators})*$")).expect("text allowlist")
}

/// Single-line text: glyphs and the space separators (Unicode Zs).
static ON_ONE_LINE: LazyLock<Regex> = LazyLock::new(|| text(r"\p{Zs}"));
/// The same, plus `\n` between lines and `\t` (CRLF is converted at the
/// board's entry).
static MARKDOWN: LazyLock<Regex> = LazyLock::new(|| text(r"[\p{Zs}\n\t]"));
/// The characters a name from outside is shown with as they are: letters,
/// marks, numbers, punctuation and symbols. Anything else (spaces,
/// controls, format characters such as bidirectional overrides,
/// zero-width and tag characters, unassigned code points) is not.
static SHOWN_CHAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\p{L}\p{M}\p{N}\p{P}\p{S}]$").expect("a valid class"));

/// Whether `c` is shown as it is ([`SHOWN_CHAR`]).
pub fn shown_char(c: char) -> bool {
    SHOWN_CHAR.is_match(c.encode_utf8(&mut [0; 4]))
}

/// Something to read: a letter, number, punctuation mark or symbol that
/// shows (not a default ignorable or the braille blank).
static VISIBLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[[\p{L}\p{N}\p{P}\p{S}]--[\p{Default_Ignorable_Code_Point}\x{2800}]]")
        .expect("visible allowlist")
});

/// Single-line text with something visible, at most `max_chars` characters.
pub fn line(field: &str, text: &str, max_chars: usize) -> Result<(), SchemaError> {
    let fits = text.chars().count() <= max_chars;
    if fits && ON_ONE_LINE.is_match(text) && VISIBLE.is_match(text) {
        Ok(())
    } else {
        Err(SchemaError::new(
            field,
            format!("must be 1 to {max_chars} visible characters on one line"),
        ))
    }
}

/// Markdown of at most `max_chars` characters; it may be empty.
pub fn markdown(field: &str, text: &str, max_chars: usize) -> Result<(), SchemaError> {
    if text.chars().count() <= max_chars && MARKDOWN.is_match(text) {
        Ok(())
    } else {
        Err(SchemaError::new(
            field,
            format!("must be at most {max_chars} characters of text, newlines and tabs"),
        ))
    }
}

/// Markdown of at most `max_chars` characters with something visible.
pub fn filled_markdown(field: &str, text: &str, max_chars: usize) -> Result<(), SchemaError> {
    markdown(field, text, max_chars)?;
    if VISIBLE.is_match(text) {
        Ok(())
    } else {
        Err(SchemaError::new(field, "must have something visible"))
    }
}

/// At most `max` entries, no two equal.
pub fn distinct<T: Ord>(field: &str, entries: &[T], max: usize) -> Result<(), SchemaError> {
    let unique: std::collections::BTreeSet<&T> = entries.iter().collect();
    if entries.len() <= max && unique.len() == entries.len() {
        Ok(())
    } else {
        Err(SchemaError::new(
            field,
            format!("must hold at most {max} distinct entries"),
        ))
    }
}

#[cfg(test)]
#[path = "schema_error_tests.rs"]
mod tests;
