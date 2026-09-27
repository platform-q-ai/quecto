//! #2211 / #2225: markup read as plain text.
use super::*;

/// #2211: Brave's highlight markup and entities read as plain text.
#[test]
fn a_snippet_reads_as_one_plain_line() {
    assert_eq!(
        inline_text(
            "A &#x27;lifetime&#x27; is <strong>how long a reference lives for</strong> &amp; more"
        ),
        "A 'lifetime' is how long a reference lives for & more"
    );
    assert_eq!(
        inline_text("Tom &amp; Jerry&#39;s <b>cartoon</b>\n  <br>next"),
        "Tom & Jerry's cartoon next"
    );
    assert_eq!(inline_text("  plain  text "), "plain text");
    assert_eq!(inline_text(""), "");
    // An escaped tag is text, not markup.
    assert_eq!(inline_text("use &lt;main&gt; once"), "use <main> once");
}

/// Only listed elements separate their text; any other adds nothing.
#[test]
fn only_listed_elements_separate() {
    assert_eq!(separator("DIV", true), Separator::Line);
    assert_eq!(separator("div", false), Separator::Line);
    assert_eq!(separator("A", true), Separator::Space);
    assert_eq!(separator("a", false), Separator::Nothing);
    for inline in [
        "span",
        "strong",
        "em",
        "abbr",
        "code",
        "b",
        "i",
        "article-x",
    ] {
        assert_eq!(separator(inline, true), Separator::Nothing, "{inline}");
    }
}

/// A word's last character always ends a label; a stop only when an
/// element closed after it (#2248 review: code punctuation such as `.` or
/// `::` right before a link is not a label's end).
#[test]
fn a_label_ends_at_a_word_or_at_a_closed_stop() {
    for last in ['x', 'É', '9'] {
        assert!(ends_a_label(Some(last), false), "{last:?}");
        assert!(ends_a_label(Some(last), true), "{last:?}");
    }
    for last in ['.', ',', ';', ':', '!', '?', ')', ']', '}'] {
        assert!(ends_a_label(Some(last), true), "{last:?}");
        assert!(!ends_a_label(Some(last), false), "{last:?}");
    }
    for last in [' ', '\n', '(', '[', '{', '"', '\'', '-', '/', '<', '>', '&'] {
        assert!(!ends_a_label(Some(last), true), "{last:?}");
        assert!(!ends_a_label(Some(last), false), "{last:?}");
    }
    assert!(!ends_a_label(None, true));
    assert!(!ends_a_label(None, false));
}

/// Every block element breaks the line, opening and closing; every
/// side-by-side one is spaced from a word before it, only when it opens.
#[test]
fn every_listed_element_separates_its_text() {
    for tag in [
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
    ] {
        assert_eq!(
            markup_to_text(&format!("x<{tag}>y</{tag}>z")),
            "x\ny\nz",
            "{tag}"
        );
    }
    for tag in ["a", "button", "label", "td", "th"] {
        assert_eq!(
            markup_to_text(&format!("x<{tag} k=v>y</{tag}>z")),
            "x yz",
            "{tag}"
        );
    }
}

/// #2248 review M1: a link inside code or identifier text is never spaced
/// from the character before it, whether that came from an entity or from
/// code punctuation; labels that run together still are, and prose reads
/// as written.
#[test]
fn a_link_in_code_is_not_spaced() {
    for (html, expected) in [
        ("Option&lt;<a>String</a>&gt;", "Option<String>"),
        ("&amp;<a>str</a>", "&str"),
        ("<a>std</a>::<a>vec</a>::<a>Vec</a>", "std::vec::Vec"),
        ("&#40;<a>paren</a>)", "(paren)"),
        ("&quot;<a>quoted</a>&quot;", "\"quoted\""),
        ("foo.<a>bar</a>()", "foo.bar()"),
        ("x:<a>y</a>; z;<a>w</a>", "x:y; z;w"),
        (
            "<pre><code>Vec&lt;<a>u8</a>&gt; = x.<a>iter</a>();</code></pre>",
            "\nVec<u8> = x.iter();\n",
        ),
        ("<code>foo<a>bar</a></code>", "foobar"),
        ("<pre>Hash<a>Map</a>::<a>new</a></pre>", "\nHashMap::new\n"),
        ("<kbd>Ctrl<a>C</a></kbd>", "CtrlC"),
        ("<samp>ok<a>go</a></samp>", "okgo"),
        ("<CODE>a<a>b</a></CODE>c<a>d</a>", "abc d"),
        (
            "Navigation<a>Introduction to Node.js</a><a>Getting Started</a>",
            "Navigation Introduction to Node.js Getting Started",
        ),
        ("see the <a>docs</a>.", "see the docs."),
        ("<span>End.</span><a>Next</a>", "End. Next"),
        ("<td>1.</td><td>2</td>", "1. 2"),
    ] {
        assert_eq!(markup_to_text(html), expected, "{html}");
    }
}

/// Code markup that never closes, or closes more often than it opens,
/// never leaves labels unspaced for the rest of the page or spaced in code.
#[test]
fn unbalanced_code_markup_is_bounded() {
    assert_eq!(markup_to_text("</code>a<a>b</a>"), "a b");
    assert_eq!(markup_to_text("<code/>a<a>b</a>"), "a b");
    assert_eq!(markup_to_text("<code>a<a>b</a>"), "ab");
}

/// Entities are decoded once, as the text is read: an escaped entity reads
/// as that entity, and an escaped tag is text.
#[test]
fn entities_are_decoded_once() {
    assert_eq!(markup_to_text("&amp;lt;b&amp;gt;"), "&lt;b&gt;");
    assert_eq!(markup_to_text("&lt;b&gt;x&lt;/b&gt;"), "<b>x</b>");
    assert_eq!(inline_text("&amp;amp;"), "&amp;");
}

/// #2248 review L4: only `<` followed by a letter, `/`, `!` or `?` opens a
/// tag; any other `<` is text. A `<` straight before a letter still reads
/// as a tag (`a<b and c>d` is markup to a browser too).
#[test]
fn a_less_than_sign_in_text_is_kept() {
    assert_eq!(inline_text("a < b and c > d"), "a < b and c > d");
    assert_eq!(inline_text("1 <2 and 3> 0"), "1 <2 and 3> 0");
    assert_eq!(inline_text("x <= y"), "x <= y");
    assert_eq!(inline_text("<3 you"), "<3 you");
    assert_eq!(inline_text("a<b and c>d"), "ad");
    assert_eq!(inline_text("a<!-- note -->b"), "ab");
    assert_eq!(inline_text("<?xml version=\"1.0\"?>x"), "x");
    assert_eq!(inline_text("a</b>c"), "ac");
}

/// Readable text never grows past its markup: every tag and entity reads
/// as no more bytes than it is written in.
#[test]
fn text_is_never_longer_than_its_markup() {
    for html in [
        "<a>b</a>", "<p>", "&#128;", "&#65536;", "&#9;", "x<a>y", "a < b", "&nbsp;",
    ] {
        assert!(markup_to_text(html).len() <= html.len(), "{html}");
    }
}
