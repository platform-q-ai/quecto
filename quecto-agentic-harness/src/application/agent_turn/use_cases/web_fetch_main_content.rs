//! Landmark-first readable text for web_fetch (#2165 part 3).
//!
//! A page that marks its main content (one `<main>`, one `role="main"`
//! element, or one `<article>`) is read from that element alone, under the
//! page's title and a one-line note, when the element holds a substantial
//! share of the page's text. Every other page reads exactly as
//! [`strip_html`](super::strip_html) reads it. One pass finds the
//! landmarks and at most three more read them: linear in the page.
use super::{
    decode_entities, find_configured_close_tag, push_collapsed_line, remove_configured_tag_blocks,
    text_of_markup,
};

/// The one line said when only the main content is kept.
pub const MAIN_CONTENT_NOTE: &str =
    "[Main content only; raw: true returns the whole page as served]";

/// A landmark must hold at least a third of the page's text to be trusted.
/// Below that it is more likely a hero banner in `<main>` or one teaser
/// `<article>` on a listing than the content, and keeping it alone would
/// drop most of what the page says; real article and documentation pages
/// measure well above it (#2165 fixtures: 68% and 69%).
const LEAST_SHARE: (usize, usize) = (1, 3);

/// A landmark holding more than nine tenths of the page's text drops too
/// little to be worth the note: the page reads as a whole.
const MOST_SHARE: (usize, usize) = (9, 10);

/// The elements `role="main"` is honoured on: containers that close.
const ROLE_MAIN_CONTAINERS: &[&str] = &["div", "section", "main", "article"];

/// Readable text for an HTML page: its main content when it marks one
/// that holds a substantial share of its text, otherwise the whole page.
pub fn readable_html(html: &str) -> String {
    let cleaned = remove_configured_tag_blocks(html);
    let whole = text_of_markup(&cleaned);
    let found = scan(&cleaned);
    let main_text = [found.main, found.role_main, found.article]
        .into_iter()
        .filter(|seen| seen.count == 1)
        .filter_map(|seen| seen.first)
        .filter_map(|open| {
            let end = content_end(&cleaned, open)?;
            debug_assert!(open.content_start <= end, "content ends before it starts");
            Some(text_of_markup(&cleaned[open.content_start..end]))
        })
        .find(|text| substantial(text.len(), whole.len()));
    match main_text {
        Some(text) => with_title(found.title, &text),
        None => whole,
    }
}

/// Whether `part` bytes of `whole` are a share worth keeping alone.
fn substantial(part: usize, whole: usize) -> bool {
    let at_least = part.saturating_mul(LEAST_SHARE.1) >= whole.saturating_mul(LEAST_SHARE.0);
    let at_most = part.saturating_mul(MOST_SHARE.1) <= whole.saturating_mul(MOST_SHARE.0);
    part > 0 && at_least && at_most
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

/// One pass over the tags: the title (before `<body>`) and the landmarks.
fn scan(html: &str) -> Found<'_> {
    let mut found = Found::default();
    let mut in_head = true;
    let mut title_sought = false;
    let mut tags = Tags { html, pos: 0 };
    while let Some(tag) = tags.next() {
        if tag.closing {
            continue;
        }
        let open = Open {
            name: tag.name,
            content_start: tags.pos,
        };
        let is = |name: &str| tag.name.eq_ignore_ascii_case(name);
        if is("body") {
            in_head = false;
        } else if is("title") && in_head && !title_sought {
            // Sought once: a page of unclosed titles stays linear.
            title_sought = true;
            if let Some(close) = find_configured_close_tag(html, tags.pos, "title") {
                found.title = Some(&html[tags.pos..close]);
                tags.pos = close;
            }
        }
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

/// Where an element's content ends: at its own close, past nested
/// elements of the same name; `None` when it never closes.
fn content_end(html: &str, open: Open<'_>) -> Option<usize> {
    let mut depth = 0_usize;
    let tags = Tags {
        html,
        pos: open.content_start,
    };
    for tag in tags {
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

/// The named tags of a page from `pos` on, past comments. Ends at an
/// unterminated comment or tag: nothing after either is a whole tag.
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
            let gt = rest.find('>')?;
            let inner = &rest[1..gt];
            // A stray `<` before a tag: resume at the last `<` before the
            // `>`, scanned at most twice, so still linear.
            if let Some(lt) = inner.rfind('<') {
                self.pos = start + 1 + lt;
                continue;
            }
            self.pos = start + gt + 1;
            let (closing, body) = match inner.strip_prefix('/') {
                Some(body) => (true, body),
                None => (false, inner),
            };
            let name_len = body
                .find(|c: char| !c.is_ascii_alphanumeric())
                .unwrap_or(body.len());
            if name_len > 0 {
                return Some(Tag {
                    start,
                    closing,
                    name: &body[..name_len],
                    attrs: &body[name_len..],
                });
            }
        }
    }
}

#[cfg(test)]
#[path = "web_fetch_main_content_tests.rs"]
mod tests;
