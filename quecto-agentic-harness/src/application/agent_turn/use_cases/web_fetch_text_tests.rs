//! #2225: adjacent block and inline-block elements do not run their text
//! together.
use super::*;

/// The issue's case: a label and the links after it, no whitespace
/// between them in the markup.
#[test]
fn adjacent_links_and_labels_are_separated() {
    let html = "<div><span>Navigation</span><a href=\"/a\">Introduction to Node.js</a><a href=\"/b\">Getting Started</a></div>";
    let text = strip_html(html);
    assert!(
        !text.contains("NavigationIntroduction") && !text.contains("Node.jsGetting"),
        "{text}"
    );
    assert_eq!(text, "Navigation Introduction to Node.js Getting Started");
}

/// Its lines, blank ones (between blocks) aside, joined by `|`.
fn lines_of(html: &str) -> String {
    let text = strip_html(html);
    let lines: Vec<&str> = text.lines().filter(|line| !line.is_empty()).collect();
    lines.join("|")
}

#[test]
fn block_elements_end_their_line() {
    for (html, expected) in [
        ("<ul><li>One</li><li>Two</li></ul>", "One\nTwo"),
        ("<section>One</section><section>Two</section>", "One\nTwo"),
        ("<aside>One</aside><main>Two</main>", "One\nTwo"),
        ("<article>One</article><article>Two</article>", "One\nTwo"),
        ("<dl><dt>Term</dt><dd>Meaning</dd></dl>", "Term\nMeaning"),
        ("<h2>Title</h2><p>Body</p>", "Title\nBody"),
        ("<figure>A<figcaption>B</figcaption></figure>", "A\nB"),
        ("<table><tr><td>A</td><td>B</td></tr></table>", "A B"),
        ("<table><tr><th>A</th><th>B</th></tr></table>", "A B"),
        ("One<br>Two<br/>Three", "One\nTwo\nThree"),
        ("One<hr>Two", "One\nTwo"),
        ("<button>Yes</button><button>No</button>", "Yes No"),
        ("<label>Name</label><input><label>Age</label>", "Name Age"),
    ] {
        assert_eq!(lines_of(html), expected.replace('\n', "|"), "{html}");
    }
}

/// Inline elements inside prose add nothing: words and punctuation keep
/// their places.
#[test]
fn inline_elements_in_prose_are_unchanged() {
    for (html, expected) in [
        ("see <a href=\"/x\">the docs</a>.", "see the docs."),
        ("(<a>link</a>)", "(link)"),
        ("un<strong>believ</strong>able", "unbelievable"),
        ("a <em>b</em> <code>c</code>", "a b c"),
        ("x<span>y</span>z", "xyz"),
        ("\"<a>quoted</a>\"", "\"quoted\""),
    ] {
        assert_eq!(strip_html(html), expected, "{html}");
    }
}

/// A link right after a word, or after an element that ended with a
/// stop, starts a new label; a stop straight before a link is code
/// punctuation (`foo.<a>bar</a>`) and is not spaced (#2248 review).
#[test]
fn a_link_after_a_word_or_a_closed_stop_is_spaced() {
    for (html, expected) in [
        ("Home<a>Docs</a>", "Home Docs"),
        ("<span>End.</span><a>Next</a>", "End. Next"),
        ("<a>A</a><a>B</a><a>C</a>", "A B C"),
        ("<b>Menu:</b><a>One</a>", "Menu: One"),
        ("<i>(x)</i><a>y</a>", "(x) y"),
        ("End.<a>Next</a>", "End.Next"),
        ("x::<a>y</a>", "x::y"),
    ] {
        assert_eq!(strip_html(html), expected, "{html}");
    }
}
