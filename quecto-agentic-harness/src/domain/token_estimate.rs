//! The local token estimate, by character class (#305, #2212).
//!
//! Pure and cheap: one pass over the characters, no allocation. The rates
//! come from the #2212 QA evidence against a provider's own count and from
//! cl100k/o200k comparisons in its review:
//!
//! - prose (ASCII words without digits, and their separators): about 4
//!   characters per token; the `docs context` page grew the provider count
//!   by 300 for an estimate of 298 (1.0x);
//! - dense (ASCII words carrying a digit, and the one separator that
//!   follows each): about 2 characters per token; `seq` output grew the
//!   provider count at 2.0x the ASCII/4 figure. Numbers, hex digests,
//!   UUIDs, versions, CSV and log columns fall here. Padding after the
//!   first separator (`cat -n`, aligned columns) is prose again;
//! - high entropy (runs of 16 or more characters of letters, digits and
//!   `+ / = - _` carrying upper case, lower case and a digit, that switch
//!   between those classes on at least 2 characters in 5): about 1.4
//!   characters per token. Base64, JWTs, API keys and random ids fall here
//!   (random base64 switches on about 65% of characters); paths and
//!   CamelCase identifiers, which switch only at word boundaries, do not;
//! - non-ASCII: about 1 token per character (CJK, emoji).
//!
//! What remains (tokeniser differences, JSON punctuation, framing) is the
//! provider-observed residual in `domain::context_calibration`.

/// ASCII prose: characters per token.
pub const PROSE_CHARS_PER_TOKEN: usize = 4;
/// ASCII dense runs (digit-bearing words and their first separator).
pub const DENSE_CHARS_PER_TOKEN: usize = 2;
/// High-entropy runs: 5 tokens per 7 characters (1.4 characters a token).
pub const HIGH_ENTROPY_TOKENS_PER_SEVEN_CHARS: usize = 5;
/// The shortest run that can be high entropy.
pub const HIGH_ENTROPY_MIN_RUN: usize = 16;
/// The fewest class switches (upper, lower, digit) per 10 characters of a
/// high-entropy run.
pub const HIGH_ENTROPY_MIN_SWITCHES_PER_TEN: usize = 4;

/// Estimate the tokens of `text` by class (see the module docs). Never
/// below [`estimate_opaque_tokens`]: every class is at least as dense as
/// ASCII/4.
pub fn estimate_tokens(text: &str) -> usize {
    let mut counts = ClassCounts::default();
    for c in text.chars() {
        counts.push(c);
    }
    let tokens = counts.finish();
    debug_assert!(tokens >= estimate_opaque_tokens(text), "density only adds");
    tokens
}

/// Estimate an opaque payload (replayed encrypted reasoning): plain
/// ASCII/4, since its length says little about its tokens. Images are
/// priced by their pixel size instead (`conversation::image_tokens`).
pub fn estimate_opaque_tokens(text: &str) -> usize {
    let (ascii, non_ascii) = text.chars().fold((0usize, 0usize), |(a, n), c| {
        if c.is_ascii() { (a + 1, n) } else { (a, n + 1) }
    });
    ascii.div_ceil(PROSE_CHARS_PER_TOKEN) + non_ascii
}

/// Characters that join the words of a high-entropy run (base64 and
/// base64url punctuation).
fn joins_a_run(c: char) -> bool {
    matches!(c, '+' | '/' | '=' | '-' | '_')
}

/// Running per-class character counts. A word (ASCII letters and digits)
/// is classed when it ends, by whether it carried a digit; the separator
/// right after a dense word is dense. A run (words joined by
/// [`joins_a_run`]) holds its words' classes provisionally until it ends:
/// a long mixed run is high entropy as a whole.
#[derive(Default)]
struct ClassCounts {
    prose: usize,
    dense: usize,
    high_entropy: usize,
    non_ascii: usize,
    word_len: usize,
    word_has_digit: bool,
    next_separator_dense: bool,
    run: Run,
}

#[derive(Default)]
struct Run {
    len: usize,
    upper: bool,
    lower: bool,
    digit: bool,
    /// Adjacent alphanumerics of different classes (upper, lower, digit).
    switches: usize,
    /// The previous character's class; `None` after a joiner.
    last_class: Option<AlnumClass>,
    prose: usize,
    dense: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AlnumClass {
    Upper,
    Lower,
    Digit,
}

impl AlnumClass {
    fn of(c: char) -> Self {
        match c {
            'A'..='Z' => Self::Upper,
            'a'..='z' => Self::Lower,
            _ => Self::Digit,
        }
    }
}

impl Run {
    fn push_alnum(&mut self, c: char) {
        debug_assert!(c.is_ascii_alphanumeric());
        let class = AlnumClass::of(c);
        self.switches += usize::from(self.last_class.is_some_and(|last| last != class));
        self.last_class = Some(class);
        self.len += 1;
        self.upper |= class == AlnumClass::Upper;
        self.lower |= class == AlnumClass::Lower;
        self.digit |= class == AlnumClass::Digit;
    }

    fn is_high_entropy(&self) -> bool {
        self.len >= HIGH_ENTROPY_MIN_RUN
            && self.upper
            && self.lower
            && self.digit
            && self.switches * 10 >= self.len * HIGH_ENTROPY_MIN_SWITCHES_PER_TEN
    }
}

impl ClassCounts {
    fn push(&mut self, c: char) {
        if c.is_ascii_alphanumeric() {
            self.word_len += 1;
            self.word_has_digit |= c.is_ascii_digit();
            self.run.push_alnum(c);
            return;
        }
        if joins_a_run(c) && self.run.len > 0 {
            self.end_word();
            self.run.len += 1;
            self.run.last_class = None;
            match self.take_separator_class() {
                true => self.run.dense += 1,
                false => self.run.prose += 1,
            }
            return;
        }
        self.end_run();
        if c.is_ascii() {
            match self.take_separator_class() {
                true => self.dense += 1,
                false => self.prose += 1,
            }
        } else {
            self.non_ascii += 1;
            self.next_separator_dense = false;
        }
    }

    /// Whether this separator is dense; only the first after a dense word.
    fn take_separator_class(&mut self) -> bool {
        std::mem::take(&mut self.next_separator_dense)
    }

    fn end_word(&mut self) {
        if self.word_len > 0 {
            match self.word_has_digit {
                true => self.run.dense += self.word_len,
                false => self.run.prose += self.word_len,
            }
            self.next_separator_dense = self.word_has_digit;
        }
        self.word_len = 0;
        self.word_has_digit = false;
    }

    fn end_run(&mut self) {
        self.end_word();
        let run = std::mem::take(&mut self.run);
        debug_assert_eq!(run.len, run.prose + run.dense, "every run char is classed");
        if run.is_high_entropy() {
            self.high_entropy += run.len;
            self.next_separator_dense = false;
        } else {
            self.prose += run.prose;
            self.dense += run.dense;
        }
    }

    fn finish(mut self) -> usize {
        self.end_run();
        self.prose.div_ceil(PROSE_CHARS_PER_TOKEN)
            + self.dense.div_ceil(DENSE_CHARS_PER_TOKEN)
            + (self.high_entropy * HIGH_ENTROPY_TOKENS_PER_SEVEN_CHARS).div_ceil(7)
            + self.non_ascii
    }
}

#[cfg(test)]
#[path = "token_estimate_tests.rs"]
mod tests;
