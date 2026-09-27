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

/// A word's last character in a script that spaces its words always ends
/// a label; once an element closed after it, any visible character does
/// (#2248 round 2: `50%` or `«` in a cell or a link). Code punctuation with
/// no element closed after it ends nothing.
#[test]
fn a_label_ends_at_a_spaced_word_or_after_a_close() {
    for last in ['x', 'Z', 'É', 'ß', '9', 'λ', 'Ж', 'ệ'] {
        assert!(ends_a_label(Some(last), false), "{last:?}");
        assert!(ends_a_label(Some(last), true), "{last:?}");
    }
    for last in [
        '.', ',', ';', ':', '!', '?', ')', ']', '}', '(', '%', '«', '»', '"', '\'', '-', '/', '<',
        '>', '&', '東', 'の', 'カ', '한', 'ไ', '×', '÷', '١',
    ] {
        assert!(ends_a_label(Some(last), true), "{last:?}");
        assert!(!ends_a_label(Some(last), false), "{last:?}");
    }
    for last in [' ', '\n', '\t', '\u{a0}', '\u{3000}', '\u{7}'] {
        assert!(!ends_a_label(Some(last), true), "{last:?}");
        assert!(!ends_a_label(Some(last), false), "{last:?}");
    }
    assert!(!ends_a_label(None, true));
    assert!(!ends_a_label(None, false));
}

/// Only letters of scripts written with spaces between words, and ASCII
/// digits, are a word's end (#2248 round 2 L3): Latin, Greek and Cyrillic.
#[test]
fn only_spaced_scripts_end_a_word() {
    for (c, spaced) in [
        ('a', true),
        ('0', true),
        ('\u{c0}', true),
        ('\u{24f}', true),
        ('\u{250}', false),
        ('\u{bf}', false),
        ('\u{370}', true),
        ('\u{3ff}', true),
        ('\u{400}', true),
        ('\u{52f}', true),
        ('\u{530}', false),
        ('\u{1e00}', true),
        ('\u{1fff}', false),
        ('\u{1ffc}', true),
        ('\u{1dff}', false),
        ('\u{2000}', false),
        ('\u{d7}', false),
        ('\u{f7}', false),
        ('東', false),
        ('ア', false),
        ('한', false),
        ('ก', false),
        ('\u{663}', false),
    ] {
        assert_eq!(ends_a_spaced_word(c), spaced, "{c:?}");
    }
}

/// #2248 round 2 L1: a cell or a link ending in any visible character is
/// spaced from the next one; code punctuation before a link, with nothing
/// closed between, is not.
#[test]
fn a_closed_label_is_spaced_whatever_it_ends_in() {
    for (html, expected) in [
        ("<td>50%</td><td>60%</td>", "50% 60%"),
        ("<a>\u{ab}</a><a>\u{bb}</a>", "\u{ab} \u{bb}"),
        ("<th>(a)</th><th>&lt;b&gt;</th>", "(a) <b>"),
        ("<a>x</a> <a>y</a>", "x y"),
        ("<a>x</a>\n<a>y</a>", "x\ny"),
        ("foo.<a>bar</a>", "foo.bar"),
        ("a(<a>b</a>)", "a(b)"),
        ("<code><a>x</a>%<a>y</a></code>", "x%y"),
    ] {
        assert_eq!(markup_to_text(html), expected, "{html}");
    }
}

/// #2248 round 2 L3: a link inside prose of a script written without
/// spaces is not spaced from it; one in a spaced script is.
#[test]
fn a_link_in_unspaced_prose_is_not_spaced() {
    for (html, expected) in [
        ("東京の<a>天気</a>", "東京の天気"),
        ("カタカナ<a>リンク</a>", "カタカナリンク"),
        ("한국<a>날씨</a>", "한국날씨"),
        ("ภาษาไทย<a>ลิงก์</a>", "ภาษาไทยลิงก์"),
        ("Straße<a>x</a>", "Straße x"),
        ("Ελλάδα<a>x</a>", "Ελλάδα x"),
        ("Москва<a>x</a>", "Москва x"),
        ("<td>東京</td><td>大阪</td>", "東京 大阪"),
    ] {
        assert_eq!(markup_to_text(html), expected, "{html}");
    }
}

/// #2248 round 2 L4: code or typed-input markup left open ends at the next
/// block or cell boundary outside `pre`; inside `pre` it holds.
#[test]
fn unclosed_code_ends_at_a_block_boundary() {
    for (html, expected) in [
        ("<code>x</p><p><a>Home</a><a>About</a>", "x\n\nHome About"),
        ("<kbd>x<div>a<a>b</a></div>", "x\na b\n"),
        ("<samp>x<li>a<a>b</a>", "x\na b"),
        ("<code>x<td>a</td><td>b</td>", "x a b"),
        ("<code>x<th>a<a>b</a></th>", "x a b"),
        (
            "<pre><code>a<div>b<a>c</a></div></code></pre>",
            "\na\nbc\n\n",
        ),
        ("<pre>a<p>b<a>c</a></p></pre>", "\na\nbc\n\n"),
        ("<code>a<pre>b</pre>c<a>d</a>", "a\nb\nc d"),
        ("<pre><code>a</pre>b<a>c</a>", "\na\nb c"),
    ] {
        assert_eq!(markup_to_text(html), expected, "{html}");
    }
}

/// #2248 round 2 Q1: a `/` ending an unquoted attribute value does not
/// make a tag self-closing; one after whitespace, a quoted value or the
/// name does.
#[test]
fn a_slash_in_an_unquoted_value_is_not_self_closing() {
    for (html, expected) in [
        ("<code data-x=/>v<a>u8</a></code>", "vu8"),
        ("<code data-x=a/>v<a>u8</a></code>", "vu8"),
        ("<code href=a/b/>v<a>u8</a>", "vu8"),
        ("<code/>a<a>b</a>", "a b"),
        ("<code />a<a>b</a>", "a b"),
        ("<code x />a<a>b</a>", "a b"),
        ("<code x=\"y\"/>a<a>b</a>", "a b"),
        ("<code x='y'/>a<a>b</a>", "a b"),
        ("<code\t/>a<a>b</a>", "a b"),
    ] {
        assert_eq!(markup_to_text(html), expected, "{html}");
    }
    for (content, self_closing) in [
        ("br/", true),
        ("br /", true),
        ("br/ ", true),
        ("code data-x=/", false),
        ("code a=b/", false),
        ("code a=\"b\"/", true),
        ("code a", false),
        ("/code", false),
        ("/br/", false),
        ("/code /", false),
    ] {
        assert_eq!(read_tag(content).self_closing, self_closing, "{content}");
    }
}

/// #2248 round 2 Q1: a closing tag of a code element that is not open
/// leaves every open one as it is.
#[test]
fn a_stray_verbatim_close_changes_nothing() {
    for (html, expected) in [
        ("<pre>a</code><a>b</a></pre>", "\nab\n"),
        ("<code>a</kbd><a>b</a></code>c<a>d</a>", "abc d"),
        ("</code></code><code>a<a>b</a></code>c<a>d</a>", "abc d"),
        ("<code><code>a</code><a>b</a></code>c<a>d</a>", "abc d"),
    ] {
        assert_eq!(markup_to_text(html), expected, "{html}");
    }
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
