//! Landmark-first readable text for web_fetch (#2165 part 3).
//!
//! A page that marks its main content (one `<main>`, one `role="main"`
//! element, or one `<article>`, tried in that order) is read from that
//! element alone, headings in its own `<header>` included, under the page's
//! title and a one-line note, when the element holds a substantial share
//! of the page's text. Every other page reads exactly as
//! [`strip_html`] reads it. One pass finds the landmarks and at most three
//! more read them: linear in the page.
use super::{
    decode_entities, find_configured_close_tag, push_collapsed_line, remove_tag_blocks, strip_html,
    text_of_markup,
};

/// The one line said when only the main content is kept.
pub const MAIN_CONTENT_NOTE: &str = "[Main content only; main_only: false returns the whole page]";

/// A landmark must hold at least a third of the page's text to be trusted.
/// Below that it is more likely a hero banner in `<main>` or one teaser
/// `<article>` on a listing than the content, and keeping it alone would
/// drop most of what the page says; real article and documentation pages
/// measure well above it (#2165 fixtures: 68% and 69%).
const LEAST_SHARE: (usize, usize) = (1, 3);

/// A landmark holding more than nine tenths of the page's text is the
/// page: it drops too little to be worth the note, so the page reads as a
/// whole (and no smaller landmark inside it is tried).
const MOST_SHARE: (usize, usize) = (9, 10);

/// The elements `role="main"` is honoured on: containers that close.
const ROLE_MAIN_CONTAINERS: &[&str] = &["div", "section", "main", "article"];

/// Blocks dropped before landmarks are sought: all of [`strip_html`]'s but
/// `<header>`, which inside a landmark holds its heading.
const LANDMARK_STRIPPED_BLOCK_TAGS: &[&str] = &["script", "style", "nav", "footer", "noscript"];

/// Elements whose content is never markup of the document: skipped when
/// seeking landmarks and their ends.
const INERT_ELEMENTS: &[&str] = &["template", "textarea", "xmp"];

/// Tags that may open a document's head; the first other tag ends it.
const HEAD_TAGS: &[&str] = &[
    "html", "head", "title", "meta", "link", "base", "style", "script", "noscript", "template",
];

/// Readable text for an HTML page: its main content when it marks one
/// that holds a substantial share of its text, otherwise the whole page.
pub fn readable_html(html: &str) -> String {
    let marked = remove_tag_blocks(html, LANDMARK_STRIPPED_BLOCK_TAGS);
    let page_text = text_of_markup(&marked).len();
    let found = scan(&marked);
    match main_text(&marked, &found, page_text) {
        Some(text) => with_title(found.title, &text),
        None => strip_html(html),
    }
}

/// How much of the page's text a landmark holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Share {
    TooSmall,
    Substantial,
    TooLarge,
}

fn share(part: usize, whole: usize) -> Share {
    let at_least = part.saturating_mul(LEAST_SHARE.1) >= whole.saturating_mul(LEAST_SHARE.0);
    let at_most = part.saturating_mul(MOST_SHARE.1) <= whole.saturating_mul(MOST_SHARE.0);
    match (part > 0 && at_least, at_most) {
        (true, true) => Share::Substantial,
        (true, false) => Share::TooLarge,
        (false, _) => Share::TooSmall,
    }
}

/// The text of the first landmark that is present, unambiguous, closed and
/// substantial. A landmark too large to be worth the note is the page:
/// nothing after it is tried.
fn main_text(marked: &str, found: &Found<'_>, page_text: usize) -> Option<String> {
    for seen in [found.main, found.role_main, found.article] {
        let Some(open) = seen.first.filter(|_| seen.count == 1) else {
            continue;
        };
        let Some(end) = content_end(marked, open) else {
            continue;
        };
        debug_assert!(open.content_start <= end, "content ends before it starts");
        let text = text_of_markup(&marked[open.content_start..end]);
        match share(text.len(), page_text) {
            Share::Substantial => return Some(text),
            Share::TooLarge => return None,
            Share::TooSmall => continue,
        }
    }
    None
}

fn with_title(title: Option<&str>, text: &str) -> String {
    let mut out = String::with_capacity(text.len() + MAIN_CONTENT_NOTE.len() + 256);
    if let Some(title) = title {
        push_collapsed_line(&mut out, &decode_entities(title));
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(MAIN_CONTENT_NOTE);
    out.push_str("\n\n");
    out.push_str(text);
    out
}

/// An open tag's element name and where its content starts.
#[derive(Clone, Copy, Debug)]
struct Open<'a> {
    name: &'a str,
    content_start: usize,
}

/// How many of one kind of landmark a page has, and the first.
#[derive(Clone, Copy, Debug, Default)]
struct Seen<'a> {
    count: usize,
    first: Option<Open<'a>>,
}
impl<'a> Seen<'a> {
    fn see(&mut self, open: Open<'a>) {
        self.count += 1;
        self.first.get_or_insert(open);
    }
}

#[derive(Debug, Default)]
struct Found<'a> {
    title: Option<&'a str>,
    main: Seen<'a>,
    role_main: Seen<'a>,
    article: Seen<'a>,
}

/// One pass over the tags: the title (while in the head) and the
/// landmarks, past the page's own `<header>` and inert content.
fn scan(html: &str) -> Found<'_> {
    let mut found = Found::default();
    let mut in_head = true;
    let mut title_sought = false;
    let mut never_closed = Vec::new();
    let mut tags = Tags { html, pos: 0 };
    while let Some(tag) = tags.next() {
        let is = |name: &str| tag.name.eq_ignore_ascii_case(name);
        // The head lasts while every tag is a head tag (`</head>` too: a
        // title after it is still the head's, as a browser parses it).
        in_head = in_head && HEAD_TAGS.iter().any(|name| is(name));
        if tag.closing {
            continue;
        }
        if is("title") && in_head && !title_sought {
            // Sought once: a page of unclosed titles stays linear.
            title_sought = true;
            if let Some(close) = find_configured_close_tag(html, tags.pos, "title") {
                found.title = Some(&html[tags.pos..close]);
                tags.pos = close;
            }
            continue;
        }
        // The page's own header holds no landmark of its content.
        if let Some(name) = ["header"]
            .iter()
            .chain(INERT_ELEMENTS)
            .copied()
            .find(|name| is(name))
        {
            skip_content(&mut tags, name, &mut never_closed);
            continue;
        }
        let open = Open {
            name: tag.name,
            content_start: tags.pos,
        };
        if is("main") {
            found.main.see(open);
        }
        if is("article") {
            found.article.see(open);
        }
        if ROLE_MAIN_CONTAINERS.iter().any(|name| is(name)) && has_role_main(tag.attrs) {
            found.role_main.see(open);
        }
    }
    found
}

/// Move `tags` past the content of the `name` element just opened. One
/// that never closes is noted, so it is sought once: linear.
fn skip_content(tags: &mut Tags<'_>, name: &'static str, never_closed: &mut Vec<&'static str>) {
    if never_closed.contains(&name) {
        return;
    }
    match find_configured_close_tag(tags.html, tags.pos, name) {
        Some(close) => tags.pos = close,
        None => never_closed.push(name),
    }
}

/// Where an element's content ends: at its own close, past nested
/// elements of the same name and inert content; `None` when it never
/// closes.
fn content_end(html: &str, open: Open<'_>) -> Option<usize> {
    let mut depth = 0_usize;
    let mut never_closed = Vec::new();
    let mut tags = Tags {
        html,
        pos: open.content_start,
    };
    while let Some(tag) = tags.next() {
        let inert = INERT_ELEMENTS
            .iter()
            .copied()
            .find(|name| tag.name.eq_ignore_ascii_case(name));
        if let (false, Some(name)) = (tag.closing, inert) {
            skip_content(&mut tags, name, &mut never_closed);
            continue;
        }
        if !tag.name.eq_ignore_ascii_case(open.name) {
            continue;
        }
        match (tag.closing, depth) {
            (false, _) => depth += 1,
            (true, 0) => return Some(tag.start),
            (true, _) => depth -= 1,
        }
    }
    None
}

/// Whether a tag's attributes say `role="main"`, in any quoting or case.
fn has_role_main(attrs: &str) -> bool {
    let lower = attrs.to_ascii_lowercase();
    lower.match_indices("role").any(|(at, _)| {
        let named = lower[..at].ends_with(|c: char| c.is_ascii_whitespace());
        let value = lower[at + "role".len()..]
            .trim_start()
            .strip_prefix('=')
            .map(|value| value.trim_start());
        let value = value.map(|value| value.strip_prefix(['"', '\'']).unwrap_or(value));
        named
            && value
                .and_then(|value| value.strip_prefix("main"))
                .is_some_and(|after| {
                    after.is_empty() || after.starts_with(['"', '\'', '/', ' ', '\t', '\n', '\r'])
                })
    })
}

/// A tag: where it starts, whether it closes, its name and attributes.
#[derive(Clone, Copy, Debug)]
struct Tag<'a> {
    start: usize,
    closing: bool,
    name: &'a str,
    attrs: &'a str,
}

/// Where a tag opened by the `<` at `rest[0]` ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TagEnd {
    /// Its `>`.
    At(usize),
    /// Another `<` came first, outside quotes: the first was stray.
    Stray(usize),
    /// Neither comes: no whole tag follows.
    Never,
}

/// Find a tag's `>`, past quoted attribute values (a quote opens a value
/// only after `=`), in one forward pass.
fn tag_end(rest: &[u8]) -> TagEnd {
    let mut quote = None;
    let mut after_equals = false;
    for (at, &byte) in rest.iter().enumerate().skip(1) {
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
    TagEnd::Never
}

/// The named tags of a page from `pos` on, past comments. Ends at an
/// unterminated comment, tag or quoted value: nothing after is a whole tag.
struct Tags<'a> {
    html: &'a str,
    pos: usize,
}
impl<'a> Iterator for Tags<'a> {
    type Item = Tag<'a>;

    fn next(&mut self) -> Option<Tag<'a>> {
        loop {
            let start = self.pos + self.html.get(self.pos..)?.find('<')?;
            let rest = &self.html[start..];
            if let Some(comment) = rest.strip_prefix("<!--") {
                self.pos = start + "<!--".len() + comment.find("-->")? + "-->".len();
                continue;
            }
            let gt = match tag_end(rest.as_bytes()) {
                TagEnd::At(gt) => gt,
                // Resume at the later `<`: every byte is read once.
                TagEnd::Stray(lt) => {
                    self.pos = start + lt;
                    continue;
                }
                TagEnd::Never => return None,
            };
            self.pos = start + gt + 1;
            let inner = &rest[1..gt];
            let (closing, body) = match inner.strip_prefix('/') {
                Some(body) => (true, body),
                None => (false, inner),
            };
            // A name runs to whitespace or `/`, and starts with a letter.
            let name_len = body
                .find(|c: char| c.is_ascii_whitespace() || c == '/')
                .unwrap_or(body.len());
            let name = &body[..name_len];
            if name.starts_with(|c: char| c.is_ascii_alphabetic()) {
                return Some(Tag {
                    start,
                    closing,
                    name,
                    attrs: &body[name_len..],
                });
            }
        }
    }
}

#[cfg(test)]
#[path = "web_fetch_main_content_tests.rs"]
mod tests;
