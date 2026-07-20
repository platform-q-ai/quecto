use super::*;

fn render_md(text: &str, width: usize) -> Vec<String> {
    let mut md = Markdown::new(text, 0);
    md.render(width)
}

#[test]
fn safe_http_link_label_is_real_osc8_hyperlink() {
    let lines = render_md("[Example](https://example.com)", 80);
    let rendered = lines.join("\n");

    assert!(
        rendered.contains("\x1b]8;;https://example.com\x07"),
        "safe links should open an OSC 8 hyperlink: {rendered:?}"
    );
    assert!(
        rendered.contains("Example\x1b[0m\x1b]8;;\x07"),
        "safe links should close OSC 8 after the label: {rendered:?}"
    );
}

#[test]
fn wrapped_long_link_reopens_and_closes_osc8_on_every_physical_line() {
    let url = "https://example.com";
    let lines = render_md("[averyveryverylonglinklabel](https://example.com)", 10);

    assert!(lines.len() > 1, "test must exercise wrapping: {lines:?}");
    let link_lines: Vec<_> = lines
        .iter()
        .filter(|line| line.contains(&format!("\x1b]8;;{url}\x07")))
        .collect();
    assert!(
        link_lines.len() > 1,
        "link label must span wrapped chunks: {lines:?}"
    );

    for line in &link_lines {
        assert!(
            line.starts_with(&format!("\x1b]8;;{url}\x07\x1b[4m\x1b[34m")),
            "every wrapped link chunk must reopen OSC 8 + SGR styling: {line:?}"
        );
        assert!(
            line.contains("\x1b[0m\x1b]8;;\x07"),
            "every wrapped link chunk must close SGR + OSC 8 before pane boundary: {line:?}"
        );
    }

    for line in lines {
        assert!(
            visible_width(&line) <= 10,
            "wrapped link line must stay clipped to chat width: {line:?}"
        );
    }
}

#[test]
fn wrapped_spaced_link_reopens_and_closes_osc8_on_word_boundary_lines() {
    let url = "https://example.com";
    let lines = render_md("[hello world friend](https://example.com)", 8);

    assert!(
        lines.len() > 1,
        "test must exercise word-boundary wrapping: {lines:?}"
    );
    let link_lines: Vec<_> = lines
        .iter()
        .filter(|line| line.contains(&format!("\x1b]8;;{url}\x07")))
        .collect();
    assert!(
        link_lines.len() > 1,
        "spaced label must span link chunks: {lines:?}"
    );

    for line in &link_lines {
        assert!(
            line.starts_with(&format!("\x1b]8;;{url}\x07\x1b[4m\x1b[34m")),
            "word-boundary continuation must reopen OSC 8 + SGR: {line:?}"
        );
        assert!(
            line.contains("\x1b[0m\x1b]8;;\x07"),
            "word-boundary line must close SGR + OSC 8 before boundary: {line:?}"
        );
    }
}
