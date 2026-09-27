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

#[test]
fn a_label_ends_at_a_word_or_a_stop() {
    for last in ['x', 'É', '9', '.', ',', ';', ':', '!', '?', ')', ']', '}'] {
        assert!(ends_a_label(Some(last)), "{last:?}");
    }
    for last in [' ', '\n', '(', '[', '{', '"', '\'', '-', '/'] {
        assert!(!ends_a_label(Some(last)), "{last:?}");
    }
    assert!(!ends_a_label(None));
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
            tags_to_text(&format!("x<{tag}>y</{tag}>z")),
            "x\ny\nz",
            "{tag}"
        );
    }
    for tag in ["a", "button", "label", "td", "th"] {
        assert_eq!(
            tags_to_text(&format!("x<{tag} k=v>y</{tag}>z")),
            "x yz",
            "{tag}"
        );
    }
}
