use super::*;

impl SelectList {
    pub(crate) fn render_text(&mut self, width: usize) -> String {
        self.render(width).join("\n")
    }
}

fn make_items(labels: &[&str]) -> Vec<SelectItem> {
    labels
        .iter()
        .map(|l| SelectItem {
            value: l.to_string(),
            label: l.to_string(),
            description: None,
        })
        .collect()
}

#[test]
fn renders_items() {
    let mut list = SelectList::new(make_items(&["A", "B", "C"]), 10);
    let lines = list.render(40);
    let joined: String = lines.join("\n");
    assert!(joined.contains("A"));
    assert!(joined.contains("B"));
    assert!(joined.contains("C"));
}

#[test]
fn selection_indicator() {
    let mut list = SelectList::new(make_items(&["A", "B"]), 10);
    let lines = list.render(40);
    // First item should have selection indicator.
    assert!(lines[0].contains("→") || lines[0].contains("→"));
}

#[test]
fn navigate_down() {
    let mut list = SelectList::new(make_items(&["A", "B", "C"]), 10);
    list.handle_input(&Key::Down);
    assert_eq!(list.selected_item().unwrap().value, "B");
}

#[test]
fn navigate_up_wraps() {
    let mut list = SelectList::new(make_items(&["A", "B", "C"]), 10);
    list.handle_input(&Key::Up);
    assert_eq!(list.selected_item().unwrap().value, "C");
}

#[test]
fn navigate_down_wraps() {
    let mut list = SelectList::new(make_items(&["A", "B", "C"]), 10);
    list.handle_input(&Key::Down);
    list.handle_input(&Key::Down);
    list.handle_input(&Key::Down);
    assert_eq!(list.selected_item().unwrap().value, "A");
}

#[test]
fn enter_selects() {
    let mut list = SelectList::new(make_items(&["A", "B"]), 10);
    list.handle_input(&Key::Down);
    list.handle_input(&Key::Enter);
    assert_eq!(list.take_result(), SelectResult::Selected("B".to_string()));
}

#[test]
fn escape_cancels() {
    let mut list = SelectList::new(make_items(&["A"]), 10);
    list.handle_input(&Key::Escape);
    assert_eq!(list.take_result(), SelectResult::Dismissed);
}

#[test]
fn empty_list() {
    let mut list = SelectList::new(vec![], 10);
    let lines = list.render(40);
    assert!(!lines.is_empty());
}

#[test]
fn with_descriptions() {
    let items = vec![SelectItem {
        value: "model".to_string(),
        label: "model".to_string(),
        description: Some("Select model".to_string()),
    }];
    let mut list = SelectList::new(items, 10);
    let lines = list.render(60);
    let joined: String = lines.join("\n");
    let plain: String = joined
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    assert!(plain.contains("model"), "should show label");
    assert!(
        plain.contains("Select model"),
        "should show description: {}",
        plain
    );
}

#[test]
fn scroll_indicator_on_overflow() {
    let mut list = SelectList::new(make_items(&["A", "B", "C", "D", "E"]), 3);
    let lines = list.render(40);
    let joined: String = lines.join("\n");
    let plain: String = joined
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    assert!(
        plain.contains("1/5"),
        "should show scroll position: {}",
        plain
    );
}

#[test]
fn sync_items_preserves_selected_value_and_clamps_when_removed() {
    let mut list = SelectList::new(make_items(&["A", "B", "C"]), 10);
    list.handle_input(&Key::Down);
    assert_eq!(list.selected_item().unwrap().value, "B");

    list.sync_items(make_items(&["C", "B", "A"]));
    assert_eq!(list.selected_item().unwrap().value, "B");
    assert_eq!(list.item_count(), 3);

    list.sync_items(make_items(&["C"]));
    assert_eq!(list.selected_item().unwrap().value, "C");
    assert_eq!(list.item_count(), 1);
}

#[test]
fn route_overlay_key_closes_only_after_terminal_result() {
    let mut slot = Some(SelectList::new(make_items(&["A", "B"]), 10));

    assert_eq!(route_overlay_key(&mut slot, &Key::Down), None);
    assert!(
        slot.is_some(),
        "pending navigation should keep overlay open"
    );

    assert_eq!(
        route_overlay_key(&mut slot, &Key::Enter),
        Some("B".to_string())
    );
    assert!(slot.is_none(), "selection should close overlay");

    let mut slot = Some(SelectList::new(make_items(&["A"]), 10));
    assert_eq!(route_overlay_key(&mut slot, &Key::Escape), None);
    assert!(slot.is_none(), "dismissal should close overlay");
}

#[test]
fn unhandled_key_is_not_consumed_and_empty_enter_stays_pending() {
    let mut list = SelectList::new(vec![], 10);

    assert!(!list.handle_input(&Key::Char('x')));
    assert!(list.handle_input(&Key::Enter));
    assert_eq!(list.take_result(), SelectResult::Pending);
}
