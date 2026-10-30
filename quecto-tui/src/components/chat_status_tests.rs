//! Ordinary status layout and diagnostic retention compatibility.
use super::*;

#[test]
fn ordinary_status_keeps_single_line_while_error_diagnostic_wraps() {
    let text = "◆ 3 sub-agents ended a turn: issue-895-feature, basic-10, basic-11. Inspect agent_cmd get_messages before treating their work as complete.";
    let mut chat = Chat::new();
    chat.add_entry(ChatEntry::Status { text: text.into() });
    let lines = chat.render(84);
    assert_eq!(
        lines.len(),
        2,
        "ordinary status retains its blank plus one line"
    );
    assert!(strip_ansi(&lines[1]).contains("Inspect agent"));
    assert!(matches!(&chat.entries()[0], ChatEntry::Status { text: retained } if retained == text));

    let diagnostic =
        format!("Error: {text}\nOriginal error retained; appended diagnostic remains accessible.");
    let mut chat = Chat::new();
    chat.add_entry(ChatEntry::Status {
        text: diagnostic.clone(),
    });
    let lines = chat.render(40);
    assert!(lines.len() > 3, "diagnostic must wrap rather than truncate");
    let visible = lines
        .iter()
        .map(|line| strip_ansi(line))
        .collect::<Vec<_>>()
        .join(" ");
    assert!(visible.contains("complete."));
    assert!(visible.contains("accessible."));
    assert!(matches!(&chat.entries()[0], ChatEntry::Status { text } if text == &diagnostic));
}
