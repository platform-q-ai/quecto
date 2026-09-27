//! Readable text from HTML markup: tags dropped (a line for a block
//! element, a space before a side-by-side one that would run into the word
//! before it; #2225), entities decoded and whitespace collapsed. Pure text
//! functions shared by web_fetch's readable views and web_search's result
//! snippets (#2211); linear in their input.

/// A fragment of inline markup (a search result's title or snippet) as one
/// plain line: tags dropped, entities decoded, whitespace collapsed
/// (#2211). Nothing is removed with its content: a snippet has no blocks.
pub fn inline_text(markup: &str) -> String {
    let mut line = String::with_capacity(markup.len());
    push_collapsed_line(&mut line, &decode_entities(&tags_to_text(markup)));
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
/// starting straight after a word or a stop is spaced from it.
const SIDE_BY_SIDE_TAGS: &[&str] = &["a", "button", "label", "td", "th"];

/// What an element's tag adds to the text around it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Separator {
    Line,
    Space,
    Nothing,
}

/// What the tag of `tag_name` adds: a line for a block element, opening or
/// closing; a space for a side-by-side element's opening tag.
fn separator(tag_name: &str, opening: bool) -> Separator {
    let is = |tags: &[&str]| tags.iter().any(|tag| tag_name.eq_ignore_ascii_case(tag));
    match (is(BLOCK_TAGS), is(SIDE_BY_SIDE_TAGS) && opening) {
        (true, _) => Separator::Line,
        (false, true) => Separator::Space,
        (false, false) => Separator::Nothing,
    }
}

/// Whether text ending in `last` runs into what follows unless spaced: a
/// word's last character or a stop. An opening bracket or quote does not.
fn ends_a_label(last: Option<char>) -> bool {
    last.is_some_and(|c| {
        c.is_alphanumeric() || matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}')
    })
}

/// Convert HTML tags to text: block tags become newlines, side-by-side
/// elements are spaced from a word before them, others are stripped.
///
/// Uses `eq_ignore_ascii_case` per tag to avoid allocating a lowercase copy
/// for every tag in the document. All text between tags is copied as UTF-8
/// substrings, so multibyte characters (e.g. `é`) are preserved.
pub fn tags_to_text(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut pos = 0;

    while let Some(open) = html[pos..].find('<') {
        let abs_open = pos + open;
        result.push_str(&html[pos..abs_open]);

        // Past quoted attribute values (#2165 review); a stray `<` is text.
        if let TagEnd::At(end_offset) = tag_end(&html.as_bytes()[abs_open..]) {
            let tag_content = &html[abs_open + 1..abs_open + end_offset];
            // An opening tag's name follows its `<` directly.
            let opening = tag_content.starts_with(|c: char| c.is_ascii_alphabetic());
            let trimmed = tag_content.trim().trim_start_matches('/');
            let tag_end = trimmed
                .find(|c: char| c.is_whitespace() || c == '/')
                .unwrap_or(trimmed.len());
            let tag_name = &trimmed[..tag_end];

            match separator(tag_name, opening) {
                Separator::Line => result.push('\n'),
                Separator::Space if ends_a_label(result.chars().next_back()) => {
                    result.push(' ');
                }
                Separator::Space | Separator::Nothing => {}
            }
            pos = abs_open + end_offset + 1;
        } else {
            // A stray or unended '<' is a literal character. The next '<'
            // is where `tag_end` stopped, so every byte is read at most
            // twice.
            result.push('<');
            pos = abs_open + 1;
        }
    }

    result.push_str(&html[pos..]);
    result
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

/// Decode common HTML entities. Operates on `&str` so multibyte characters
/// are preserved instead of being re-interpreted as Latin-1 bytes.
/// Longest entity text looked for, `&` to `;` (`&thetasym;` is 10).
const MAX_ENTITY_BYTES: usize = 12;

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
