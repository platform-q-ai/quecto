use super::*;

fn strip_ansi(s: &str) -> String {
    let mut result = String::new();
    let mut in_escape = false;
    for ch in s.chars() {
        if in_escape {
            if ch.is_ascii_alphabetic() || ch == '~' {
                in_escape = false;
            }
        } else if ch == '\x1b' {
            in_escape = true;
        } else {
            result.push(ch);
        }
    }
    result
}

#[test]
fn jump_to_latest_resets_scroll_offset_and_restores_following() {
    let mut chat = Chat::new();
    for i in 0..20 {
        chat.add_entry(ChatEntry::User {
            text: format!("line {i}"),
        });
    }
    chat.set_viewport_height(3);
    chat.render(80);
    chat.scroll_up(10);
    chat.render(80);
    assert!(chat.scroll_offset() > 0);

    chat.scroll_to_latest();
    assert_eq!(chat.scroll_offset(), 0);
    chat.add_entry(ChatEntry::Assistant {
        text: "new output".into(),
        thinking: Vec::new(),
        streaming: true,
    });
    let visible = chat
        .render(80)
        .iter()
        .map(|line| strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(chat.scroll_offset(), 0);
    assert!(visible.contains("new output"), "{visible}");
}
