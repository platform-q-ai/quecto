use super::*;

fn selector() -> EffortSelector {
    EffortSelector::new(&["none", "low", "medium", "high", "xhigh"], Some("medium"))
}

fn plain(lines: Vec<String>) -> String {
    lines.join("\n").replace("\u{1b}", "ESC")
}

#[test]
fn renders_empty_query_with_current_level_marker() {
    let mut selector = selector();

    let text = plain(selector.render(80));

    assert!(
        text.contains("Search: _"),
        "query placeholder missing: {text}"
    );
    assert!(text.contains("●"), "current level marker missing: {text}");
    assert!(
        text.contains("medium"),
        "current level label missing: {text}"
    );
}

#[test]
fn typing_filters_and_backspace_restores_matches() {
    let mut selector = selector();

    assert!(selector.handle_input(&Key::Char('x')));
    assert_eq!(selector.visible_levels(), vec!["xhigh".to_string()]);
    let filtered = plain(selector.render(80));
    assert!(
        filtered.contains("Search: x"),
        "non-empty query not rendered: {filtered}"
    );

    assert!(selector.handle_input(&Key::Backspace));
    assert_eq!(
        selector.visible_levels(),
        vec!["none", "low", "medium", "high", "xhigh"]
    );
}

#[test]
fn enter_selects_highlighted_level_after_navigation() {
    let mut selector = selector();

    assert!(selector.handle_input(&Key::Down));
    assert!(selector.handle_input(&Key::Enter));

    assert_eq!(
        selector.take_result(),
        EffortSelectorResult::Selected("low".to_string())
    );
}

#[test]
fn up_wraps_to_last_level_and_selects_it() {
    let mut selector = selector();

    assert!(selector.handle_input(&Key::Up));
    assert!(selector.handle_input(&Key::Enter));

    assert_eq!(
        selector.take_result(),
        EffortSelectorResult::Selected("xhigh".to_string())
    );
}

#[test]
fn escape_dismisses_and_unhandled_key_is_not_consumed() {
    let mut selector = selector();

    assert!(!selector.handle_input(&Key::Tab));
    assert_eq!(selector.take_result(), EffortSelectorResult::Pending);

    assert!(selector.handle_input(&Key::Escape));
    assert_eq!(selector.take_result(), EffortSelectorResult::Dismissed);
}

#[test]
fn no_match_renders_message_and_enter_dismisses() {
    let mut selector = selector();

    assert!(selector.handle_input(&Key::Char('z')));
    assert!(selector.visible_levels().is_empty());
    let text = plain(selector.render(80));
    assert!(
        text.contains("No matching levels"),
        "no-match message missing: {text}"
    );

    assert!(selector.handle_input(&Key::Enter));
    assert_eq!(selector.take_result(), EffortSelectorResult::Dismissed);
}

#[test]
fn query_is_capped_at_max_length() {
    let mut selector = EffortSelector::new(&["aaaaaaaaaaaaaaaa", "aaaaaaaaaaaaaaaaa"], None);

    for _ in 0..20 {
        assert!(selector.handle_input(&Key::Char('a')));
    }

    let text = plain(selector.render(120));
    assert!(
        text.contains("Search: aaaaaaaaaaaaaaaa"),
        "query should be capped at 16 chars: {text}"
    );
    assert!(
        !text.contains("aaaaaaaaaaaaaaaaaESC"),
        "17th query char should not be accepted: {text}"
    );
}
