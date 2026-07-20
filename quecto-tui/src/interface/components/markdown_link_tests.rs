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
fn wrapped_link_lines_end_with_sgr_reset_to_prevent_style_bleed() {
    let lines = render_md("[averyveryverylonglinklabel](https://example.com)", 10);

    assert!(lines.len() > 1, "test must exercise wrapping: {lines:?}");
    for line in lines {
        assert!(
            line.ends_with("\x1b[0m"),
            "styled wrapped link line must end reset to avoid pane bleed: {line:?}"
        );
        assert!(
            visible_width(&line) <= 10,
            "wrapped link line must stay clipped to chat width: {line:?}"
        );
    }
}
