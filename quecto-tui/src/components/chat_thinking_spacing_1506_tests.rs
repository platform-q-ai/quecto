use super::*;
use crate::components::ansi::strip_ansi;

fn plain_render(chat: &mut Chat, width: usize) -> String {
    chat.render(width)
        .iter()
        .map(|line| strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn consecutive_thinking_trace_blocks_are_compact() {
    let mut chat = Chat::new();
    chat.add_entry(ChatEntry::Assistant {
        text: String::new(),
        thinking: vec!["Inspecting message handling outcomes".into()],
        streaming: false,
    });
    chat.add_entry(ChatEntry::Assistant {
        text: "I checked its state.".into(),
        thinking: vec!["Notifying user about continued active state despite failure".into()],
        streaming: false,
    });

    let plain = plain_render(&mut chat, 80);

    assert!(
        plain.contains(
            "│ Inspecting message handling outcomes\n│ Notifying user about continued active state despite failure"
        ),
        "adjacent thinking traces should not be separated by blank lines:\n{plain}"
    );
    assert!(
        plain.contains("failure\n\nI checked its state."),
        "thinking-to-answer transition should keep one clear blank separator:\n{plain}"
    );
}

#[test]
fn wrapped_thinking_trace_blocks_are_compact() {
    let mut chat = Chat::new();
    chat.add_entry(ChatEntry::Assistant {
        text: "Done".into(),
        thinking: vec![
            "alpha beta gamma delta epsilon zeta eta theta".into(),
            "iota kappa lambda mu nu xi omicron pi".into(),
        ],
        streaming: false,
    });

    let plain = plain_render(&mut chat, 18);
    let lines: Vec<_> = plain.lines().collect();
    let first_trace_last = lines
        .iter()
        .position(|line| line.contains("theta"))
        .unwrap();

    assert!(
        lines
            .get(first_trace_last + 1)
            .is_some_and(|line| line.starts_with('│')),
        "wrapped adjacent thinking traces should remain compact:\n{plain}"
    );
}
