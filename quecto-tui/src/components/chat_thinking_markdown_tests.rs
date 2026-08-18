use super::*;
use crate::components::ansi::strip_ansi;

#[test]
fn thinking_markdown_uses_assistant_renderer_inside_border() {
    let mut chat = Chat::new();
    chat.add_entry(ChatEntry::Assistant {
        text: "Answer".into(),
        thinking: vec!["**Plan**\n\n- one\n- `two`".into()],
        streaming: false,
    });

    let lines = chat.render(80);
    let plain = lines
        .iter()
        .map(|l| strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n");

    assert!(plain.contains("│ Plan"));
    assert!(plain.contains("│ • one"));
    assert!(plain.contains("two"));
    assert!(plain.contains("\n\nAnswer"));
    let plan_line = lines
        .iter()
        .find(|line| line.contains("Plan"))
        .expect("bold thinking line");
    assert!(
        plan_line.contains("\x1b[1m"),
        "markdown bold should be preserved"
    );
    assert!(
        plan_line.contains("\x1b[0m\x1b[3m\x1b[38;2;128;128;128m"),
        "thinking style should resume after markdown resets"
    );
    assert!(
        plain.contains("`two`"),
        "markdown inline code text should be preserved"
    );
}

#[test]
fn thinking_markdown_wraps_within_bordered_width() {
    let mut chat = Chat::new();
    chat.add_entry(ChatEntry::Assistant {
        text: "Done".into(),
        thinking: vec!["- alpha beta gamma delta".into()],
        streaming: false,
    });

    for line in chat.render(12) {
        assert!(visible_width(&line) <= 12, "line too wide: {line:?}");
    }
}
