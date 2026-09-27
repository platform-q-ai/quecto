//! Landmark-first readable text for web_fetch (#2165 part 3).
//!
//! A page that marks its main content (one `<main>`, one `role="main"`
//! element, or one `<article>`, tried in that order) is read from that
//! element alone, headings in its own `<header>` included, under the page's
//! title and a one-line note saying how much text outside it was left out,
//! when the element holds at least a third of the page's text, however much
//! more (#2225: most documentation pages keep over nine tenths of their text
//! in `<main>`, and their chrome is still worth dropping). Every other page
//! reads exactly as
//! [`strip_html`] reads it. One pass finds the landmarks and at most three
//! more read them: linear in the page.
use super::{find_configured_close_tag, remove_tag_blocks, strip_html, text_of_markup};
use crate::domain::html_text::{TagEnd, decode_entities, push_collapsed_line, tag_end};

/// How the one line said when only the main content is kept starts.
pub const MAIN_CONTENT_NOTE_LEAD: &str = "[Main content only; ";

/// The one line said when only the main content is kept: how many bytes of
/// the page's readable text were dropped, and how to get them.
pub fn main_content_note(dropped_bytes: usize) -> String {
    format!(
        "{MAIN_CONTENT_NOTE_LEAD}{dropped_bytes} bytes of page text dropped; main_only: false returns the whole page]"
    )
}

/// The bytes of `text` that are not whitespace: text measured alike however
/// its lines and blocks were separated.
fn text_bytes(text: &str) -> usize {
    text.bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .count()
}

/// A landmark must hold at least a third of the page's text to be trusted.
/// Below that it is more likely a hero banner in `<main>` or one teaser
/// `<article>` on a listing than the content, and keeping it alone would
/// drop most of what the page says; real article and documentation pages
/// measure well above it (#2165 fixtures: 68% and 69%).
const LEAST_SHARE: (usize, usize) = (1, 3);

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
    // The page's share is measured on the text the whole page reads as;
    // `marked` keeps `<header>` only to slice the landmark from.
    let whole = strip_html(html);
    let marked = remove_tag_blocks(html, LANDMARK_STRIPPED_BLOCK_TAGS);
    let found = scan(&marked);
    let Some(main) = main_text(&marked, &found, whole.len()) else {
        return whole;
    };
    let title = found.title.map(title_line).filter(|line| !line.is_empty());
    // The whole page's text beyond the title and the landmark (measured as
    // the page is): what is dropped.
    let shown = main.text_bytes + title.as_deref().map_or(0, text_bytes);
    match text_bytes(&whole).saturating_sub(shown) {
        // A landmark holding all the text drops nothing: the whole page,
        // with no note (#2248 review).
        0 => whole,
        dropped => with_title(title.as_deref(), dropped, &main.text),
    }
}

/// The kept landmark: its text, and its [`text_bytes`] measured as the
/// page is.
#[derive(Debug)]
struct MainText {
    text: String,
    text_bytes: usize,
}

/// Whether a landmark holding `part` of the page's `whole` text holds
/// enough of it to be trusted: at least [`LEAST_SHARE`], and something.
fn is_substantial(part: usize, whole: usize) -> bool {
    part > 0 && part.saturating_mul(LEAST_SHARE.1) >= whole.saturating_mul(LEAST_SHARE.0)
}

/// The text of the first landmark that is present, unambiguous, closed and
/// substantial.
fn main_text(marked: &str, found: &Found<'_>, page_text: usize) -> Option<MainText> {
    for seen in [found.main, found.role_main, found.article] {
        let Some(open) = seen.first.filter(|_| seen.count == 1) else {
            continue;
        };
        let Some(end) = content_end(marked, open) else {
            continue;
        };
        debug_assert!(open.content_start <= end, "content ends before it starts");
        let content = &marked[open.content_start..end];
        // Measured as the page is, its nested `<header>` stripped (#2222
        // review); returned with it.
        let measured = strip_html(content);
        if is_substantial(measured.len(), page_text) {
            return Some(MainText {
                text: text_of_markup(content),
                text_bytes: text_bytes(&measured),
            });
        }
    }
    None
}

/// A title as its line reads: entities decoded, whitespace collapsed.
fn title_line(title: &str) -> String {
    let mut line = String::with_capacity(title.len());
    push_collapsed_line(&mut line, &decode_entities(title));
    line
}

fn with_title(title: Option<&str>, dropped_bytes: usize, text: &str) -> String {
    let note = main_content_note(dropped_bytes);
    let mut out = String::with_capacity(text.len() + note.len() + 256);
    if let Some(title) = title {
        out.push_str(title);
        out.push('\n');
    }
    out.push_str(&note);
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

/// Whether a tag's attributes hold one named exactly `role` whose first
/// token is `main`, in any case. Attributes are walked as a browser reads
/// them: a name, then optionally `=` and a quoted or unquoted value, so
/// `role=main` inside another attribute's value is not one (#2165 review).
fn has_role_main(attrs: &str) -> bool {
    let space = |c: char| c.is_ascii_whitespace();
    let mut rest = attrs;
    loop {
        rest = rest.trim_start_matches(|c: char| space(c) || c == '/');
        if rest.is_empty() {
            return false;
        }
        let before = rest.len();
        let name_len = rest
            .find(|c: char| space(c) || c == '=' || c == '/')
            .unwrap_or(rest.len());
        let name = &rest[..name_len];
        rest = rest[name_len..].trim_start_matches(space);
        let mut value = "";
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start_matches(space);
            (value, rest) = match after.chars().next() {
                Some(quote @ ('"' | '\'')) => {
                    let body = &after[1..];
                    match body.find(quote) {
                        Some(end) => (&body[..end], &body[end + 1..]),
                        None => (body, ""),
                    }
                }
                _ => after.split_at(after.find(space).unwrap_or(after.len())),
            };
        }
        debug_assert!(rest.len() < before, "an attribute read nothing");
        let first_token = value.split_ascii_whitespace().next();
        if name.eq_ignore_ascii_case("role")
            && first_token.is_some_and(|token| token.eq_ignore_ascii_case("main"))
        {
            return true;
        }
    }
}

/// A tag: where it starts, whether it closes, its name and attributes.
#[derive(Clone, Copy, Debug)]
struct Tag<'a> {
    start: usize,
    closing: bool,
    name: &'a str,
    attrs: &'a str,
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
#[path = "web_fetch_main_content_share_tests.rs"]
mod share_tests;
#[cfg(test)]
#[path = "web_fetch_main_content_tests.rs"]
mod tests;
