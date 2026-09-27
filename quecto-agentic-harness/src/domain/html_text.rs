//! Readable text from HTML markup: tags dropped (a line for a block
//! element, a space before a side-by-side one that would run into the label
//! before it; #2225), entities decoded as the text is read and whitespace
//! collapsed. Pure text functions shared by web_fetch's readable views and
//! web_search's result snippets (#2211); linear in their input.

/// A fragment of inline markup (a search result's title or snippet) as one
/// plain line: tags dropped, entities decoded, whitespace collapsed
/// (#2211). Nothing is removed with its content: a snippet has no blocks.
pub fn inline_text(markup: &str) -> String {
    let mut line = String::with_capacity(markup.len());
    push_collapsed_line(&mut line, &markup_to_text(markup));
    line
}

/// Elements that open and close a line of their own (#2225): their text
/// never runs into the next element's.
const BLOCK_TAGS: &[&str] = &[
    "p",
    "div",
    "li",
    "tr",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "blockquote",
    "pre",
    "br",
    "hr",
    "ul",
    "ol",
    "dl",
    "dt",
    "dd",
    "section",
    "article",
    "aside",
    "main",
    "nav",
    "header",
    "footer",
    "table",
    "thead",
    "tbody",
    "tfoot",
    "caption",
    "figure",
    "figcaption",
    "details",
    "summary",
    "form",
    "fieldset",
    "legend",
    "address",
    "option",
];

/// Elements laid out side by side, each its own label or cell (#2225): one
/// starting straight after a label is spaced from it.
const SIDE_BY_SIDE_TAGS: &[&str] = &["a", "button", "label", "td", "th"];

/// Elements whose text is code or typed input (#2248 review): a link in it
/// is part of an identifier or an expression, never spaced from it.
const VERBATIM_TAGS: &[&str] = &["pre", "code", "kbd", "samp"];

/// Characters that end a label when an element closes right after them: a
/// label reading `Home.` or `Menu:`. Straight before a link, with no
/// element closed between, they are code punctuation (`foo.<a>bar</a>`,
/// `std::<a>vec</a>`) and end nothing.
const CLOSED_STOPS: &[char] = &['.', ',', ';', ':', '!', '?', ')', ']', '}'];

/// What an element's tag adds to the text around it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Separator {
    Line,
    Space,
    Nothing,
}

/// Whether `tag_name` is one of `tags`, whatever its case.
fn is_one_of(tags: &[&str], tag_name: &str) -> bool {
    tags.iter().any(|tag| tag_name.eq_ignore_ascii_case(tag))
}

/// What the tag of `tag_name` adds: a line for a block element, opening or
/// closing; a space for a side-by-side element's opening tag.
fn separator(tag_name: &str, opening: bool) -> Separator {
    match (
        is_one_of(BLOCK_TAGS, tag_name),
        is_one_of(SIDE_BY_SIDE_TAGS, tag_name) && opening,
    ) {
        (true, _) => Separator::Line,
        (false, true) => Separator::Space,
        (false, false) => Separator::Nothing,
    }
}

/// Whether text whose last decoded character is `last` runs into a label
/// that follows unless spaced: a word's last letter or digit always does;
/// a stop only when an element `closed` after it (#2248 review).
fn ends_a_label(last: Option<char>, closed: bool) -> bool {
    last.is_some_and(|c| c.is_alphanumeric() || (closed && CLOSED_STOPS.contains(&c)))
}

/// Whether the `<` that starts `rest` may open a tag: only when a letter
/// (an element), `/` (its end), `!` (a comment or doctype) or `?` (a
/// processing instruction) follows (#2248 review). Any other `<`, as in
/// `a < b` or `<3`, is text.
fn opens_a_tag(rest: &[u8]) -> bool {
    debug_assert_eq!(rest.first(), Some(&b'<'), "not at a `<`");
    rest.get(1)
        .is_some_and(|next| next.is_ascii_alphabetic() || matches!(next, b'/' | b'!' | b'?'))
}

/// Readable text as it is built: the text so far, whether an element
/// closed since its last character, and how deep in verbatim elements.
#[derive(Debug, Default)]
struct ReadableText {
    text: String,
    closed_since_text: bool,
    verbatim_depth: usize,
}

impl ReadableText {
    /// Text between tags, its entities decoded now so that what a label
    /// ends in is the character shown, not its escape.
    fn push_text(&mut self, raw: &str) {
        if raw.is_empty() {
            return;
        }
        self.text.push_str(&decode_entities(raw));
        self.closed_since_text = false;
    }

    /// The tag of `tag_name`: `opening` or closing, `self_closing` or not.
    fn push_tag(&mut self, tag_name: &str, opening: bool, self_closing: bool) {
        if is_one_of(VERBATIM_TAGS, tag_name) && !self_closing {
            // Unbalanced markup never goes below the page's own level.
            self.verbatim_depth = match opening {
                true => self.verbatim_depth.saturating_add(1),
                false => self.verbatim_depth.saturating_sub(1),
            };
        }
        match separator(tag_name, opening) {
            Separator::Line => self.text.push('\n'),
            Separator::Space if self.spaces_a_label() => self.text.push(' '),
            Separator::Space | Separator::Nothing => {}
        }
        if !opening {
            self.closed_since_text = true;
        }
    }

    /// Whether a side-by-side element opening now is spaced from the text
    /// before it: outside code, after a label's end.
    fn spaces_a_label(&self) -> bool {
        self.verbatim_depth == 0
            && ends_a_label(self.text.chars().next_back(), self.closed_since_text)
    }
}

/// Convert HTML markup to text: block tags become newlines, side-by-side
/// elements are spaced from a label before them, others are stripped, and
/// entities are decoded (once, as the text is read).
///
/// Uses `eq_ignore_ascii_case` per tag to avoid allocating a lowercase copy
/// for every tag in the document. All text between tags is copied as UTF-8
/// substrings, so multibyte characters (e.g. `é`) are preserved.
pub fn markup_to_text(html: &str) -> String {
    let mut out = ReadableText {
        text: String::with_capacity(html.len()),
        ..ReadableText::default()
    };
    let mut pos = 0;

    while let Some(open) = html[pos..].find('<') {
        let abs_open = pos + open;
        out.push_text(&html[pos..abs_open]);
        let rest = &html.as_bytes()[abs_open..];

        // Past quoted attribute values (#2165 review); a stray `<` is text,
        // and one that cannot open a tag is never scanned as one.
        match opens_a_tag(rest).then(|| tag_end(rest)) {
            Some(TagEnd::At(end_offset)) => {
                let tag_content = &html[abs_open + 1..abs_open + end_offset];
                // An opening tag's name follows its `<` directly.
                let opening = tag_content.starts_with(|c: char| c.is_ascii_alphabetic());
                let self_closing = tag_content.trim_end().ends_with('/');
                let trimmed = tag_content.trim().trim_start_matches('/');
                let name_end = trimmed
                    .find(|c: char| c.is_whitespace() || c == '/')
                    .unwrap_or(trimmed.len());
                out.push_tag(&trimmed[..name_end], opening, self_closing);
                pos = abs_open + end_offset + 1;
            }
            Some(TagEnd::Stray(_) | TagEnd::Never) | None => {
                // A stray, unended or unopened '<' is a literal character.
                // The next '<' is where `tag_end` stopped, so every byte is
                // read at most twice.
                out.push_text("<");
                pos = abs_open + 1;
            }
        }
    }

    out.push_text(&html[pos..]);
    // Every tag reads as at most one character and every entity as no more
    // bytes than its escape: text never outgrows its markup.
    assert!(
        out.text.len() <= html.len(),
        "readable text outgrew its markup"
    );
    out.text
}

/// Where a tag opened by the `<` at `rest[0]` ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagEnd {
    /// Its `>`.
    At(usize),
    /// Another `<` came first, outside quotes: the first was stray.
    Stray(usize),
    /// Neither follows: no whole tag does.
    Never,
}

/// Find a tag's `>` in one forward pass, past quoted attribute values (a
/// quote opens a value only after `=`; #2165 review).
///
/// A quote that never closes is no value: the tag then ends at the first
/// `>`, or the next starts at the first `<`, after its start, whatever the
/// quotes. Both lie past that quote, and that quote character appears
/// nowhere after it, so no later tag reads to the end for it again: at
/// most two such reads a page, one per quote character, so linear.
pub fn tag_end(rest: &[u8]) -> TagEnd {
    let mut quote = None;
    let mut after_equals = false;
    let mut first_bracket = None;
    for (at, &byte) in rest.iter().enumerate().skip(1) {
        if first_bracket.is_none() {
            first_bracket = match byte {
                b'>' => Some(TagEnd::At(at)),
                b'<' => Some(TagEnd::Stray(at)),
                _ => None,
            };
        }
        if let Some(open) = quote {
            if byte == open {
                quote = None;
            }
            continue;
        }
        match byte {
            b'>' => return TagEnd::At(at),
            b'<' => return TagEnd::Stray(at),
            b'"' | b'\'' if after_equals => {
                quote = Some(byte);
                after_equals = false;
                continue;
            }
            _ => {}
        }
        after_equals = byte == b'=' || (after_equals && byte.is_ascii_whitespace());
    }
    // Brackets inside values that closed were values; only an unclosed
    // quote falls back to the first bracket.
    match quote {
        Some(_) => first_bracket.unwrap_or(TagEnd::Never),
        None => TagEnd::Never,
    }
}

/// Longest entity text looked for, `&` to `;` (`&thetasym;` is 10).
const MAX_ENTITY_BYTES: usize = 12;

/// Decode common HTML entities. Operates on `&str` so multibyte characters
/// are preserved instead of being re-interpreted as Latin-1 bytes.
pub fn decode_entities(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut pos = 0;

    while let Some(amp) = text[pos..].find('&') {
        let abs_amp = pos + amp;
        result.push_str(&text[pos..abs_amp]);

        // An entity is short: look for its `;` only within reach, so many
        // `&` with a far `;` stay linear (#2177 review).
        let reach = text.len().min(abs_amp + MAX_ENTITY_BYTES);
        let window = text.get(abs_amp..reach).unwrap_or("");
        if let Some(semi) = window.find(';') {
            let entity = &text[abs_amp + 1..abs_amp + semi];
            if let Some(decoded) = decode_entity(entity) {
                result.push(decoded);
                pos = abs_amp + semi + 1;
                continue;
            }
        }
        result.push('&');
        pos = abs_amp + 1;
    }

    result.push_str(&text[pos..]);
    result
}

/// Decode a single HTML entity (without & and ;).
fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" | "#39" => Some('\''),
        "nbsp" => Some(' '),
        _ if entity.starts_with('#') => {
            let num_str = &entity[1..];
            if let Some(hex) = num_str
                .strip_prefix('x')
                .or_else(|| num_str.strip_prefix('X'))
            {
                u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
            } else {
                num_str.parse::<u32>().ok().and_then(char::from_u32)
            }
        }
        _ => None,
    }
}

/// Append `line` to `out` with runs of whitespace collapsed to single spaces,
/// leading/trailing whitespace trimmed. No per-line allocation.
pub fn push_collapsed_line(out: &mut String, line: &str) {
    let mut prev_space = true; // true = trim leading spaces
    for ch in line.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    // Trim trailing space
    if out.ends_with(' ') {
        out.pop();
    }
}

#[cfg(test)]
#[path = "html_text_tests.rs"]
mod tests;
