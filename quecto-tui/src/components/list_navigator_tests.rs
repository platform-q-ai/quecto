use super::*;

#[test]
fn next_wraps_and_previous_wraps() {
    let mut nav = ListNavigator::new();
    nav.move_previous(3);
    assert_eq!(nav.selected(), 2);
    nav.move_next(3);
    assert_eq!(nav.selected(), 0);
}

#[test]
fn empty_lists_keep_selection_at_zero() {
    let mut nav = ListNavigator::new();
    nav.move_next(0);
    assert_eq!(nav.selected(), 0);
    nav.move_previous(0);
    assert_eq!(nav.selected(), 0);
}

#[test]
fn clamp_keeps_selected_in_bounds() {
    let mut nav = ListNavigator::new();
    nav.move_previous(5);
    assert_eq!(nav.selected(), 4);
    nav.clamp(2);
    assert_eq!(nav.selected(), 1);
    nav.clamp(0);
    assert_eq!(nav.selected(), 0);
}

#[test]
fn visible_range_scrolls_to_selected_item() {
    let mut nav = ListNavigator::new();
    for _ in 0..4 {
        nav.move_next(10);
    }
    assert_eq!(nav.selected(), 4);
    assert_eq!(nav.visible_range(10, 3), 2..5);
    assert_eq!(nav.visible_range(2, 10), 0..2);
    assert_eq!(nav.visible_range(0, 3), 0..0);
}
