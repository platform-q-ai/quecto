use super::*;

#[test]
fn test_definition() {
    let tool = WebFetchTool::new(32);
    let def = tool.definition();
    assert_eq!(def.name.as_ref(), "web_fetch");
    assert!(def.description.contains("Fetch"));
}

// ─── HTML stripping ──────────────────────────────────────────────────────

#[test]
fn test_strip_html_basic() {
    let html = "<p>Hello <b>world</b></p>";
    let text = strip_html(html);
    assert!(text.contains("Hello world"), "got: {text}");
}

#[test]
fn test_strip_html_removes_script() {
    let html = "<p>Before</p><script>alert('xss')</script><p>After</p>";
    let text = strip_html(html);
    assert!(text.contains("Before"));
    assert!(text.contains("After"));
    assert!(!text.contains("alert"));
}

#[test]
fn test_strip_html_removes_style() {
    let html = "<style>.foo { color: red; }</style><p>Content</p>";
    let text = strip_html(html);
    assert!(!text.contains("color"));
    assert!(text.contains("Content"));
}

#[test]
fn test_strip_html_removes_noscript() {
    let html = "<p>Before</p><noscript>Hidden</noscript><p>After</p>";
    let text = strip_html(html);
    assert!(text.contains("Before"));
    assert!(text.contains("After"));
    assert!(!text.contains("Hidden"));
}

#[test]
fn test_strip_html_removes_nav_footer_header() {
    let html = "<nav>Menu</nav><main>Content</main><footer>Copyright</footer>";
    let text = strip_html(html);
    assert!(!text.contains("Menu"));
    assert!(text.contains("Content"));
    assert!(!text.contains("Copyright"));
}

#[test]
fn test_strip_html_block_newlines() {
    let html = "<p>First</p><p>Second</p>";
    let text = strip_html(html);
    assert!(text.contains("First\n"), "got: {text:?}");
    assert!(text.contains("Second"));
}

#[test]
fn test_strip_html_br() {
    let html = "Line 1<br>Line 2<br/>Line 3";
    let text = strip_html(html);
    assert!(text.contains("Line 1\n"), "got: {text:?}");
    assert!(text.contains("Line 2\n"), "got: {text:?}");
}

#[test]
fn test_strip_html_collapses_whitespace() {
    let html = "<p>  lots   of    spaces  </p>";
    let text = strip_html(html);
    assert_eq!(text, "lots of spaces");
}

#[test]
fn test_strip_html_multiline_collapse() {
    let html = "<p>A</p>\n\n\n\n\n<p>B</p>";
    let text = strip_html(html);
    assert!(!text.contains("\n\n\n"), "got: {text:?}");
}

#[test]
fn test_strip_html_list_items() {
    let html = "<ul><li>One</li><li>Two</li><li>Three</li></ul>";
    let text = strip_html(html);
    assert!(text.contains("One"));
    assert!(text.contains("Two"));
    assert!(text.contains("Three"));
}

#[test]
fn test_strip_html_headings() {
    let html = "<h1>Title</h1><p>Paragraph</p>";
    let text = strip_html(html);
    assert!(text.contains("Title"));
    assert!(text.contains("Paragraph"));
}

#[test]
fn test_strip_html_plain_text_passthrough() {
    let text = strip_html("Just plain text, no HTML.");
    assert_eq!(text, "Just plain text, no HTML.");
}

#[test]
fn test_strip_html_removes_configured_tags_case_insensitively() {
    let html = "<HEADER>Top</HEADER><p>Keep</p><NoScript>Hidden</NoScript><NAV>Menu</NAV>";
    let text = strip_html(html);
    assert_eq!(text, "Keep");
}

#[test]
fn test_strip_html_removes_script_case_insensitive() {
    let html = "<SCRIPT>bad</SCRIPT>good";
    let result = strip_html(html);
    assert!(!result.contains("bad"));
    assert!(result.contains("good"));
}

#[test]
fn test_strip_html_removes_script_with_attributes() {
    let html = r#"<script type="text/javascript">bad</script>good"#;
    let result = strip_html(html);
    assert!(!result.contains("bad"));
    assert!(result.contains("good"));
}

// ─── Entity decoding ─────────────────────────────────────────────────────

#[test]
fn test_decode_entity_named() {
    assert_eq!(decode_entity("amp"), Some('&'));
    assert_eq!(decode_entity("lt"), Some('<'));
    assert_eq!(decode_entity("gt"), Some('>'));
    assert_eq!(decode_entity("quot"), Some('"'));
    assert_eq!(decode_entity("apos"), Some('\''));
    assert_eq!(decode_entity("nbsp"), Some(' '));
}

#[test]
fn test_decode_entity_numeric() {
    assert_eq!(decode_entity("#65"), Some('A'));
    assert_eq!(decode_entity("#x41"), Some('A'));
    assert_eq!(decode_entity("#x2603"), Some('☃'));
}

#[test]
fn test_decode_entities_in_text() {
    assert_eq!(decode_entities("&amp; &lt; &gt;"), "& < >");
    assert_eq!(decode_entities("hello&nbsp;world"), "hello world");
    assert_eq!(decode_entities("&#65;"), "A");
}

#[test]
fn test_tags_to_text_preserves_non_ascii() {
    let html = "<p>café résumé naïve</p>";
    assert_eq!(tags_to_text(html), "\ncafé résumé naïve\n");
}

#[test]
fn test_decode_entities_preserves_non_ascii() {
    assert_eq!(decode_entities("&#233;"), "é");
    assert_eq!(decode_entities("café"), "café");
}

#[test]
fn test_truncate_utf8_ascii() {
    assert_eq!(truncate_utf8("hello world", 5), "hello");
}

#[test]
fn test_truncate_utf8_boundary() {
    let s = "café";
    assert_eq!(truncate_utf8(s, 4), "caf");
    assert_eq!(truncate_utf8(s, 5), "café");
}

#[test]
fn test_truncate_utf8_no_truncation() {
    assert_eq!(truncate_utf8("short", 100), "short");
}

#[path = "web_fetch_security_tests.rs"]
mod security;
