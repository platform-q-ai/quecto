//! #2165 part 3: landmark-first extraction, measured on real-world-shaped
//! pages.
use super::super::strip_html;
use super::*;

const ARTICLE: &str =
    include_str!("../../../../tests/fixtures/web_fetch/article_with_sidebar.html");
const DOCS: &str = include_str!("../../../../tests/fixtures/web_fetch/docs_with_main.html");
const NO_LANDMARKS: &str = include_str!("../../../../tests/fixtures/web_fetch/no_landmarks.html");

/// The fixture's article text: the part between its `article-start` and
/// `article-end` comments, headings in its `<header>` included (renamed to
/// a `<div>` here, so the whole-page stripper keeps them).
fn article_lines(fixture: &str) -> Vec<String> {
    let start = fixture
        .find("<!-- article-start -->")
        .expect("start marker");
    let end = fixture.find("<!-- article-end -->").expect("end marker");
    let article = fixture[start..end]
        .replace("<header", "<div")
        .replace("</header>", "</div>");
    strip_html(&article)
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

/// (share of the output that is article text, share of the article kept).
fn measure(fixture: &str, output: &str) -> (f64, f64) {
    let lines = article_lines(fixture);
    let total: usize = lines.iter().map(String::len).sum();
    let kept: usize = lines
        .iter()
        .filter(|line| output.contains(line.as_str()))
        .map(String::len)
        .sum();
    (
        kept as f64 / output.len() as f64,
        kept as f64 / total as f64,
    )
}

/// How every note starts; its count of omitted bytes follows.
const NOTE: &str = "[Main content only; ";

/// The docs page keeps its `<main>`: the title, then the note, then the
/// article, without the sidebar, the table of contents or the languages.
#[test]
fn a_docs_page_keeps_its_main_content() {
    let text = readable_html(DOCS);
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("Array.prototype.map() - JavaScript | MDN")
    );
    assert!(lines.next().is_some_and(|line| line.starts_with(NOTE)));
    assert!(
        text.lines().any(|line| line == "Array.prototype.map()"),
        "lost the h1: {text}"
    );
    for line in article_lines(DOCS) {
        assert!(text.contains(&line), "lost {line:?}");
    }
    for chrome in [
        "Array.prototype.copyWithin()",
        "Deutsch",
        "In this article",
        "Skip to search",
    ] {
        assert!(!text.contains(chrome), "kept {chrome:?}");
    }
}

/// The blog post keeps its `<main>` (the post and its comments) and loses
/// the sidebar.
#[test]
fn an_article_page_loses_its_sidebar() {
    let text = readable_html(ARTICLE);
    assert!(
        text.lines()
            .any(|line| line == "A weekday sourdough that fits around work"),
        "lost the h1: {text}"
    );
    assert!(
        text.starts_with("A weekday sourdough that fits around work \u{2013} The Crumb Diaries\n"),
        "{text}"
    );
    for line in article_lines(ARTICLE) {
        assert!(text.contains(&line), "lost {line:?}");
    }
    for chrome in [
        "Archives",
        "Join 4,212 other subscribers",
        "Kit I use",
        "Skip to content",
    ] {
        assert!(!text.contains(chrome), "kept {chrome:?}");
    }
}

/// A page with no landmark reads exactly as before.
#[test]
fn a_page_without_landmarks_is_unchanged() {
    assert_eq!(readable_html(NO_LANDMARKS), strip_html(NO_LANDMARKS));
}

/// Landmark-first extraction raises the article's share of the output on
/// the landmark pages, and loses none of the article, its heading
/// included, anywhere. Measured: article page 41.5% -> 59.1% (8028 -> 5639
/// bytes), docs page 63.5% -> 89.9% (7574 -> 5380 bytes), page without
/// landmarks 71.6% unchanged.
#[test]
fn the_article_share_rises_where_there_is_a_landmark() {
    for (fixture, least) in [(ARTICLE, 0.55), (DOCS, 0.80), (NO_LANDMARKS, 0.70)] {
        let (share, kept) = measure(fixture, &readable_html(fixture));
        assert!(share >= least, "share {share}");
        assert!(kept > 0.999, "kept {kept}");
    }
}

fn page(body: &str) -> String {
    format!("<html><head><title>T</title></head><body>{body}</body></html>")
}

fn words(n: usize, word: &str) -> String {
    format!("<p>{}</p>", vec![word; n].join(" "))
}

/// A landmark holding under a third of the page's text is not trusted.
#[test]
fn a_small_landmark_is_not_trusted() {
    for (inside, outside, trusted) in [(30, 70, false), (37, 63, true)] {
        let html = page(&format!(
            "<main>{}</main>{}",
            words(inside, "in"),
            words(outside, "ot")
        ));
        let text = readable_html(&html);
        match trusted {
            false => assert_eq!(text, strip_html(&html), "{inside}"),
            true => assert!(
                text.contains(NOTE) && !text.contains("ot"),
                "{inside}: {text}"
            ),
        }
    }
}

/// #2225: a landmark holding nearly all the text is still kept alone, the
/// note saying how much was left out.
#[test]
fn a_landmark_holding_nearly_everything_is_kept() {
    for (inside, outside) in [(93, 7), (98, 2), (85, 15)] {
        let html = page(&format!(
            "<main>{}</main>{}",
            words(inside, "in"),
            words(outside, "ot")
        ));
        let text = readable_html(&html);
        assert!(
            text.contains(NOTE) && !text.contains("ot"),
            "{inside}: {text}"
        );
    }
}

/// More than one `<main>` or `<article>` is ambiguous: the whole page, even
/// when the first alone would be substantial.
#[test]
fn ambiguous_landmarks_keep_the_whole_page() {
    for body in [
        format!(
            "<main>{}</main><main>{}</main>{}",
            words(80, "a"),
            words(40, "b"),
            words(40, "c")
        ),
        format!(
            "<article>{}</article><article>{}</article>{}",
            words(80, "a"),
            words(40, "b"),
            words(40, "c")
        ),
        format!(
            "<div role=\"main\">{}</div><div role=\"main\">{}</div>{}",
            words(80, "a"),
            words(40, "b"),
            words(40, "c")
        ),
    ] {
        let html = page(&body);
        assert_eq!(readable_html(&html), strip_html(&html), "{body}");
    }
}

/// `<main>` wins over `role="main"`, which wins over a single `<article>`;
/// an unqualified one gives way to the next.
#[test]
fn landmarks_are_tried_in_order() {
    let side = words(40, "side");
    let html = page(&format!(
        "{side}<main>{}<article>{}</article></main>",
        words(40, "m"),
        words(80, "a")
    ));
    let text = readable_html(&html);
    assert!(
        text.contains(" m m") && text.contains(" a a") && !text.contains("side"),
        "{text}"
    );

    let html = page(&format!(
        "<main>{}</main><div role=\"main\">{}</div>",
        words(60, "m"),
        words(50, "r")
    ));
    let text = readable_html(&html);
    assert!(text.contains(" m m") && !text.contains(" r r"), "{text}");

    let html = page(&format!(
        "<section role=\"main\">{}</section><article>{}</article>",
        words(100, "r"),
        words(90, "a")
    ));
    let text = readable_html(&html);
    assert!(text.contains(" r r") && !text.contains(" a a"), "{text}");

    let html = page(&format!(
        "{side}<main>{}</main><article>{}</article>",
        words(5, "m"),
        words(80, "a")
    ));
    let text = readable_html(&html);
    assert!(
        text.contains(" a a") && !text.contains(" m m") && !text.contains("side"),
        "{text}"
    );
}

/// A stray `<` in the text does not hide the landmark after it.
#[test]
fn a_stray_bracket_does_not_hide_a_landmark() {
    let html = page(&format!(
        "{}<p>1 < 2 <main>{}</main>",
        words(20, "side"),
        words(60, "in")
    ));
    let text = readable_html(&html);
    assert!(text.contains(NOTE) && !text.contains("side"), "{text}");
}

/// A page with no text reads as empty: no note over nothing.
#[test]
fn an_empty_page_reads_as_empty() {
    assert_eq!(readable_html("<main> </main>"), "");
}

/// `role="main"` on a `<div>` ends at its own close, past nested divs, in
/// any quoting and case.
#[test]
fn role_main_ends_at_its_own_close() {
    for open in [
        "<div role=\"main\">",
        "<DIV class=x ROLE='Main'>",
        "<div id=a role=main>",
    ] {
        let html = page(&format!(
            "{open}<div>{}</div><div><div>{}</div></div></div>{}",
            words(30, "in"),
            words(30, "deep"),
            words(20, "out")
        ));
        let text = readable_html(&html);
        assert!(
            text.contains(" in in") && text.contains(" deep deep") && !text.contains("out"),
            "{open}: {text}"
        );
    }
}

/// A `role` that is not `main`, an attribute merely containing "role", or
/// `role="main"` on an element that is not a container, is no landmark.
#[test]
fn only_role_main_is_a_landmark() {
    for open in [
        "<div role=\"navigation\">",
        "<div data-role=\"main\">",
        "<div role=\"mainly\">",
        "<span role=\"main\">",
    ] {
        let name = open[1..].split(' ').next().unwrap_or_default();
        let html = page(&format!(
            "{open}{}</{name}>{}",
            words(60, "in"),
            words(20, "out")
        ));
        assert_eq!(readable_html(&html), strip_html(&html), "{open}");
    }
}

/// A landmark never closed, or only named in a comment or a script, is
/// no landmark.
#[test]
fn unclosed_commented_or_scripted_landmarks_are_ignored() {
    for body in [
        format!("<main>{}{}", words(60, "in"), words(20, "out")),
        format!(
            "<!-- <main> -->{}<!-- </main> -->{}",
            words(60, "in"),
            words(20, "out")
        ),
        format!(
            "<script>var s = '<main>';</script>{}<script>'</main>'</script>{}",
            words(60, "in"),
            words(20, "out")
        ),
    ] {
        let html = page(&body);
        assert_eq!(readable_html(&html), strip_html(&html), "{body}");
    }
}

/// Without a title in the head (an icon's title is none), the note leads.
#[test]
fn without_a_title_the_note_leads() {
    let html = format!(
        "<body><svg><title>icon</title></svg>{}<main>{}</main></body>",
        words(20, "side"),
        words(60, "in")
    );
    let text = readable_html(&html);
    assert!(
        text.starts_with(NOTE) && text.contains("]\n\nin in"),
        "{text}"
    );
}

/// Pages of unclosed or unterminated markup are read in linear time.
#[test]
fn malformed_pages_read_in_linear_time() {
    for page in [
        "<main><div role=main><article>x".repeat(7_000),
        "<div role=\"main\"><div>x".repeat(10_000),
        format!("<main>{}", "<div>x".repeat(35_000)),
        format!("<main>x</main>{}", "<!--".repeat(50_000)),
        format!("<title>{}", "<p>x".repeat(50_000)),
        "<title>x".repeat(30_000),
        "<".repeat(200_000),
    ] {
        assert!(page.len() >= 200_000, "{}", page.len());
        let started = std::time::Instant::now();
        let _ = readable_html(&page);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "took {:?}",
            started.elapsed()
        );
    }
}

/// A tag's name is all of it: `<main-nav>`, `<article-card>` and
/// `</main-menu>` are not `<main>` or `<article>`.
#[test]
fn a_tag_name_is_matched_whole() {
    for body in [
        format!(
            "<main-nav>{}</main-nav><main>{}</main>",
            words(40, "side"),
            words(60, "in")
        ),
        format!(
            "<article-card>{}</article-card><article>{}</article>",
            words(40, "side"),
            words(60, "in")
        ),
    ] {
        let text = readable_html(&page(&body));
        assert!(
            text.contains(NOTE) && !text.contains("side"),
            "{body}: {text}"
        );
    }
    let html = page(&format!(
        "<main>{}</main-menu>{}</main>{}",
        words(60, "in"),
        words(20, "still"),
        words(30, "out")
    ));
    let text = readable_html(&html);
    assert!(
        text.contains("still still") && !text.contains("out"),
        "{text}"
    );
}

/// #2225: a `<main>` or `role="main"` holding everything is kept whole, a
/// smaller `<article>` inside it not winning over it, and nothing is
/// omitted.
#[test]
fn a_landmark_holding_everything_omits_nothing() {
    for open in ["<main>", "<div role=\"main\">"] {
        let close = if open == "<main>" {
            "</main>"
        } else {
            "</div>"
        };
        let html = page(&format!(
            "{open}{}<article>{}</article>{}{close}",
            words(15, "intro"),
            words(80, "a"),
            words(15, "comment")
        ));
        // Nothing dropped: the whole page, with no note (#2248 review).
        let text = readable_html(&html);
        assert_eq!(text, strip_html(&html), "{open}");
        assert!(!text.contains(MAIN_CONTENT_NOTE_LEAD), "{open}: {text}");
        for kept in ["intro", "a a", "comment"] {
            assert!(text.contains(kept), "{open}: {text}");
        }
    }
}

/// A `<header>` inside the chosen landmark is content (its heading); the
/// page's own header outside it still goes.
#[test]
fn a_header_inside_the_landmark_is_kept() {
    for (open, close) in [
        ("<main>", "</main>"),
        ("<article>", "</article>"),
        ("<div role=\"main\">", "</div>"),
    ] {
        let html = page(&format!(
            "<header><p>Site banner</p></header>{}{open}<header><h1>The heading</h1></header>{}{close}",
            words(30, "side"),
            words(60, "in")
        ));
        let text = readable_html(&html);
        assert!(
            text.lines().any(|line| line == "The heading"),
            "{open}: {text}"
        );
        assert!(
            !text.contains("Site banner") && !text.contains("side"),
            "{open}: {text}"
        );
    }
}

/// A landmark inside the page's own header is not counted.
#[test]
fn a_landmark_inside_the_page_header_is_not_counted() {
    let html = page(&format!(
        "<header><article>{}</article></header><article>{}</article>{}",
        words(5, "teaser"),
        words(60, "in"),
        words(20, "side")
    ));
    let text = readable_html(&html);
    assert!(text.contains(NOTE) && !text.contains("side"), "{text}");
}

/// Without `<body>`, the head ends at the first tag that is not a head
/// tag: a `<title>` in an icon after it is not the page's title.
#[test]
fn the_head_ends_at_the_first_body_tag() {
    let html = format!(
        "<html><head><meta charset=utf-8><link rel=x></head><div>{}<svg><title>icon</title></svg></div><main>{}</main></html>",
        words(20, "side"),
        words(60, "in")
    );
    let text = readable_html(&html);
    assert!(
        text.starts_with(NOTE) && text.contains("]\n\nin in"),
        "{text}"
    );
    let html = format!(
        "<html><meta charset=utf-8><title>Kept</title><div>{}</div><main>{}</main></html>",
        words(20, "side"),
        words(60, "in")
    );
    assert!(readable_html(&html).starts_with("Kept\n"), "{html}");
}

/// A title closed with space before its `>` is kept.
#[test]
fn a_title_closed_with_space_is_kept() {
    for close in ["</title >", "</title\n>", "</TITLE\t>"] {
        let html = format!(
            "<html><head><title>Spaced</title{}<body>{}<main>{}</main></body></html>",
            &close[7..],
            words(20, "side"),
            words(60, "in")
        );
        let text = readable_html(&html);
        assert!(text.starts_with("Spaced\n"), "{close:?}: {text}");
    }
}

/// A `>` inside a quoted attribute value does not end the tag.
#[test]
fn a_quoted_bracket_does_not_end_a_tag() {
    let html = page(&format!(
        "<div data-tip=\"a > b\" role=\"main\">{}</div>{}",
        words(60, "in"),
        words(20, "side")
    ));
    let text = readable_html(&html);
    assert!(
        text.contains(NOTE) && !text.contains("side") && !text.contains("b\""),
        "{text}"
    );
    let html = page(&format!(
        "<main data-x='a>b'>{}</main>{}",
        words(60, "in"),
        words(20, "side")
    ));
    let text = readable_html(&html);
    assert!(text.contains(NOTE) && !text.contains("b'"), "{text}");
}

/// Landmarks inside `<template>`, `<textarea>` or `<xmp>` are not counted.
#[test]
fn landmarks_in_templates_and_raw_text_are_not_counted() {
    for (open, close) in [
        ("<template>", "</template>"),
        ("<textarea name=t>", "</textarea>"),
        ("<xmp>", "</xmp>"),
    ] {
        let html = page(&format!(
            "{open}<main>x</main>{close}<main>{}</main>{}",
            words(60, "in"),
            words(20, "side")
        ));
        let text = readable_html(&html);
        assert!(
            text.contains(NOTE) && !text.contains("side"),
            "{open}: {text}"
        );
    }
}

/// The new parsing paths stay linear on malformed pages.
#[test]
fn new_parsing_paths_read_in_linear_time() {
    for page in [
        format!("<main data-x=\"{}", "a>b ".repeat(60_000)),
        "<div a='x>".repeat(25_000),
        "<textarea>x".repeat(20_000),
        "<template><xmp>x".repeat(15_000),
        "<header>x".repeat(25_000),
        format!("<title>t{}", "</title ".repeat(25_000)),
        "<meta><link>".repeat(20_000),
        "<main-nav>x</main-menu>".repeat(10_000),
    ] {
        assert!(page.len() >= 200_000, "{}", page.len());
        let started = std::time::Instant::now();
        let _ = readable_html(&page);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "took {:?} on {:?}",
            started.elapsed(),
            &page[..24]
        );
    }
}

/// A quote opens an attribute value only after `=`: an apostrophe in text
/// after a stray `<` hides nothing, and `= "..."` with spaces is a value.
#[test]
fn quotes_open_values_only_after_equals() {
    let html = page(&format!(
        "{}<p>1 < 2 isn't much</p><main>{}</main>",
        words(20, "side"),
        words(60, "in")
    ));
    let text = readable_html(&html);
    assert!(text.contains(NOTE) && !text.contains("side"), "{text}");
    // Even when a later apostrophe would close it, the landmark between
    // is not swallowed.
    let html = page(&format!(
        "{}<p>1 < 2 isn't much</p><main>{}</main><p>don't</p>",
        words(20, "side"),
        words(60, "in")
    ));
    let text = readable_html(&html);
    assert!(text.contains(NOTE) && !text.contains("side"), "{text}");
    let html = page(&format!(
        "<div data-tip = \"a > b\" role=\"main\">{}</div>{}",
        words(60, "in"),
        words(20, "side")
    ));
    let text = readable_html(&html);
    assert!(text.contains(NOTE) && !text.contains("side"), "{text}");
}

/// A close tag inside inert content does not end the landmark.
#[test]
fn inert_content_does_not_end_a_landmark() {
    let html = page(&format!(
        "<main>{}<textarea>x</main>y</textarea>{}</main>{}",
        words(40, "in"),
        words(20, "more"),
        words(20, "side")
    ));
    let text = readable_html(&html);
    assert!(
        text.contains("more more") && !text.contains("side"),
        "{text}"
    );
}

/// A `<title>` after `</head>` still belongs to the head (as a browser
/// parses it): only a tag that is not a head tag ends the head.
#[test]
fn a_title_after_the_head_close_is_still_the_title() {
    let html = format!(
        "<html><head><meta charset=utf-8></head><title>After</title><div>{}</div><main>{}</main></html>",
        words(20, "side"),
        words(60, "in")
    );
    let text = readable_html(&html);
    assert!(text.starts_with("After\n"), "{text}");
}

/// Navigation, footers, scripts and styles inside the landmark still go.
#[test]
fn chrome_inside_the_landmark_still_goes() {
    let html = page(&format!(
        "{}<main><nav>Menu</nav><style>.x{{}}</style>{}<script>var s;</script><footer>Legal</footer></main>",
        words(20, "side"),
        words(60, "in")
    ));
    let text = readable_html(&html);
    assert!(text.contains(NOTE) && text.contains("in in"), "{text}");
    for chrome in ["Menu", ".x", "var s", "Legal", "side"] {
        assert!(!text.contains(chrome), "kept {chrome:?}: {text}");
    }
}

/// The page's share is measured on the text the whole page reads as: its
/// own `<header>`, which that drops, does not count against the landmark.
#[test]
fn the_page_header_does_not_count_against_the_landmark() {
    let html = page(&format!(
        "<header>{}</header><aside>{}</aside><main>{}</main>",
        words(80, "hd"),
        words(60, "sd"),
        words(40, "in")
    ));
    let text = readable_html(&html);
    assert!(text.contains(NOTE) && text.contains("in in"), "{text}");
    assert!(!text.contains("sd") && !text.contains("hd"), "{text}");
}

/// Only an attribute named `role` whose first token is `main` makes a
/// landmark; `role=main` inside another attribute's value does not.
#[test]
fn role_main_is_an_attribute_not_a_substring() {
    for (open, landmark) in [
        ("<div title=\"a role=main\">", false),
        ("<div title=\"a role=main b\">", false),
        ("<div title='x role=\"main\"'>", false),
        ("<div data-x=role=main>", false),
        ("<div role=\"main region\">", true),
        ("<div role=\"region main\">", false),
        ("<div hidden ROLE = MAIN>", true),
        ("<div role=main/>", false),
    ] {
        let html = page(&format!(
            "{open}{}</div>{}",
            words(60, "in"),
            words(20, "out")
        ));
        let text = readable_html(&html);
        assert_eq!(text.contains(NOTE), landmark, "{open}: {text}");
    }
    // A quoted `role=main` beside a real one is not a second landmark.
    let html = page(&format!(
        "<p title=\"see role=main\">{}</p><div role=\"main\">{}</div>",
        words(20, "out"),
        words(60, "in")
    ));
    let text = readable_html(&html);
    assert!(text.contains(NOTE) && !text.contains("out"), "{text}");
}

/// A quote that never closes, after a stray `<`, does not end the scan:
/// the landmarks after it are still found.
#[test]
fn an_unterminated_quote_does_not_hide_later_landmarks() {
    let html = page(&format!(
        "{}<p>x < y = \"z</p><main>{}</main>",
        words(20, "side"),
        words(60, "in")
    ));
    let text = readable_html(&html);
    assert!(text.contains(NOTE) && !text.contains("side"), "{text}");
    let text = strip_html("<p>x < y = \"z</p><p>after</p>");
    assert_eq!(text, "x < y = \"z\n\nafter");
    // A quote that closes holds its brackets: with no `>` after it, the
    // `<` was text.
    assert_eq!(
        strip_html("<p>a</p><a title=\"x>y\""),
        "a\n<a title=\"x>y\""
    );
}

/// A quoted `>` inside an attribute leaks nothing, whole page or landmark.
#[test]
fn a_quoted_bracket_leaks_no_attribute_text() {
    assert_eq!(strip_html("<p><a title=\"p>q\">link</a></p>"), "link");
    let html = page(&format!(
        "<main><a title=\"p>q\">link</a>{}</main>{}",
        words(60, "in"),
        words(20, "side")
    ));
    let text = readable_html(&html);
    assert!(text.contains(NOTE) && text.contains("link"), "{text}");
    assert!(!text.contains("q\""), "{text}");
}

/// The quote and attribute paths stay linear on malformed pages, whole
/// page and landmark alike.
#[test]
fn quote_and_attribute_paths_read_in_linear_time() {
    for page in [
        format!("<a x=\"{}", "<b y='z>".repeat(25_000)),
        "< y = \"z".repeat(25_000),
        "<p a=\"x>".repeat(25_000),
        format!("<div {}>", "role=x ".repeat(30_000)),
        "<div title=\"a role=main\">".repeat(8_000),
        format!("<div role={}>", "=".repeat(200_000)),
    ] {
        assert!(page.len() >= 200_000, "{}", page.len());
        let started = std::time::Instant::now();
        let _ = readable_html(&page);
        let _ = strip_html(&page);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "took {:?} on {:?}",
            started.elapsed(),
            &page[..24]
        );
    }
}
