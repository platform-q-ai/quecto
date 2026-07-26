use super::autocomplete::Autocomplete;
use super::chat::Chat;
use super::editor::Editor;
use super::effort_selector::EffortSelector;
use super::files_autocomplete::FilesAutocomplete;
use super::footer::Footer;
use super::markdown::Markdown;
use super::model_selector::ModelSelector;
use super::notification::{Notification, NotificationStack, NotifyLevel};
use super::select_list::{SelectItem, SelectList};
use crate::components::component::Component;
use crate::shell::keys::Key;

#[test]
fn component_defaults_construct_and_render_smoke() {
    let mut chat = Chat::default();
    assert!(chat.render(24).is_empty());
    assert!(!chat.handle_input(&Key::Ctrl('c')));

    let mut editor = Editor::default();
    assert!(!editor.render(24).is_empty());

    let mut footer = Footer::default();
    assert!(!footer.render(24).is_empty());
    assert_eq!(footer.model(), "unknown");
    assert!(!footer.handle_input(&Key::Ctrl('c')));

    let mut stack = NotificationStack::default();
    assert!(stack.render(24).is_empty());
    assert!(!stack.handle_input(&Key::Ctrl('c')));

    let mut stdin = crate::interface::stdin_buffer::StdinBuffer::default();
    assert!(stdin.drain_all_events().is_empty());

    assert_eq!(
        crate::interface::utils::sanitize_truncate_chars_with_ellipsis("abc", 2, "…"),
        "ab…"
    );
}

#[test]
fn component_invalidate_batch_is_safe_before_and_after_render() {
    let mut components: Vec<Box<dyn Component>> = vec![
        Box::new(Autocomplete::new(Vec::new(), 4)),
        Box::new(Chat::new()),
        Box::new(EffortSelector::new(
            &["low", "medium", "high"],
            Some("medium"),
        )),
        Box::new(FilesAutocomplete::new(4)),
        Box::new(Footer::new()),
        Box::new(Markdown::new("cached text", 0)),
        Box::new(ModelSelector::new(Some("openai/gpt-5"))),
        Box::new(Notification::new("notice", NotifyLevel::Info)),
        Box::new(NotificationStack::new()),
        Box::new(SelectList::new(
            vec![
                SelectItem {
                    value: "one".into(),
                    label: "one".into(),
                    description: None,
                },
                SelectItem {
                    value: "two".into(),
                    label: "two".into(),
                    description: None,
                },
            ],
            4,
        )),
    ];

    for component in &mut components {
        component.invalidate();
        let _ = component.render(32);
        component.invalidate();
        assert!(!component.handle_input(&Key::Ctrl('c')));
        let rerendered = component.render(32);
        assert!(
            rerendered
                .iter()
                .all(|line| crate::interface::utils::visible_width(line) <= 32)
        );
    }
}
