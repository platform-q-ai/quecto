use crate::components::fuzzy::fuzzy_filter;

use super::*;

impl FilesAutocomplete {
    /// Whether a background load is currently pending (tests only).
    fn is_loading(&self) -> bool {
        self.loading
    }
}

fn fa() -> FilesAutocomplete {
    FilesAutocomplete::with_files(
        vec![
            "src/main.rs".to_string(),
            "src/lib.rs".to_string(),
            "README.md".to_string(),
            "docs/workflow.md".to_string(),
        ],
        5,
    )
}

#[test]
fn activates_on_at() {
    let mut f = fa();
    f.update("@", 1);
    assert!(f.is_active());
    assert_eq!(f.list.len(), 4, "@ alone lists all files");
    assert_eq!(f.token_start(), Some(0));
}

#[test]
fn first_activation_requests_load_and_shows_loading_without_enumerating() {
    let mut f = FilesAutocomplete::new(5);
    f.update("@", 1);
    assert!(f.is_active());
    assert!(f.is_loading());
    assert!(f.take_load_request());
    assert_eq!(f.list.len(), 1);
    assert_eq!(f.list.suggestions()[0].value, "loading files…");
}

#[test]
fn loaded_files_replace_loading_state_on_next_update() {
    let mut f = FilesAutocomplete::new(5);
    f.update("@main", 5);
    assert!(f.take_load_request());
    f.apply_loaded_files(vec!["src/main.rs".to_string(), "src/lib.rs".to_string()]);
    f.update("@main", 5);
    assert!(!f.is_loading());
    assert!(f.is_active());
    assert_eq!(f.list.suggestions()[0].value, "src/main.rs");
}

#[test]
fn stale_activation_requests_reload_without_dropping_loaded_suggestions() {
    let mut f = FilesAutocomplete::with_files(vec!["src/main.rs".to_string()], 5);
    f.mark_loaded_at_for_test(Instant::now() - CACHE_TTL - Duration::from_secs(1));
    f.update("@", 1);
    assert!(f.take_load_request());
    assert!(f.is_loading());
    assert!(f.is_active());
    assert_eq!(f.list.suggestions()[0].value, "src/main.rs");
}

#[test]
fn fuzzy_filters_on_prefix() {
    let mut f = fa();
    f.update("@main", 5);
    assert!(f.is_active());
    assert!(
        f.list.suggestions()[0].value.contains("main.rs"),
        "best match should be main.rs: {:?}",
        f.list.suggestions()
    );
}

#[test]
fn inactive_without_at() {
    let mut f = fa();
    f.update("hello world", 11);
    assert!(!f.is_active());
    assert_eq!(f.token_start(), None);
}

#[test]
fn deactivates_when_token_has_whitespace() {
    let mut f = fa();
    f.update("@src done", 9);
    assert!(!f.is_active(), "a space ends the @token");
}

#[test]
fn does_not_fire_on_email_like_at() {
    let mut f = fa();
    // `@` preceded by a non-space char (e.g. an email) must not trigger.
    f.update("user@host", 9);
    assert!(!f.is_active());
}

#[test]
fn fires_mid_line_after_space() {
    let mut f = fa();
    f.update("see @src/li", 11);
    assert!(f.is_active());
    assert_eq!(f.token_start(), Some(4));
    assert!(f.list.suggestions()[0].value.contains("lib.rs"));
}

#[test]
fn tab_selects_full_path() {
    let mut f = fa();
    f.update("@main", 5);
    f.handle_input(&Key::Tab);
    match f.take_result() {
        AutocompleteResult::Selected(v) => assert_eq!(v, "src/main.rs"),
        other => panic!("expected Selected, got {other:?}"),
    }
}

#[test]
fn tab_does_not_select_loading_placeholder() {
    let mut f = FilesAutocomplete::new(5);
    f.update("@", 1);
    f.handle_input(&Key::Tab);
    assert_eq!(f.take_result(), AutocompleteResult::Pending);
    assert!(f.is_active());
}

#[test]
fn escape_dismisses() {
    let mut f = fa();
    f.update("@", 1);
    f.handle_input(&Key::Escape);
    assert_eq!(f.take_result(), AutocompleteResult::Dismissed);
    assert!(!f.is_active());
}

#[test]
fn navigate_down_changes_selection() {
    let mut f = fa();
    f.update("@", 1);
    assert_eq!(f.list.selected(), 0);
    f.handle_input(&Key::Down);
    assert_eq!(f.list.selected(), 1);
}

#[test]
fn renders_active_suggestions() {
    let mut f = fa();
    f.update("@", 1);
    let lines = f.render(60);
    assert!(!lines.is_empty());
    let plain: String = lines
        .join("\n")
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    assert!(plain.contains("@src/main.rs"), "should list files: {plain}");
}

#[test]
fn renders_nothing_when_inactive() {
    let mut f = fa();
    assert!(f.render(60).is_empty());
}

#[test]
fn cache_reload_policy() {
    let ttl = Duration::from_secs(30);
    // Injected (test) lists never reload, regardless of age.
    assert!(!should_reload(true, None, ttl));
    assert!(!should_reload(true, Some(Duration::from_secs(999)), ttl));
    // Production: load when never loaded, keep while fresh, reload when stale.
    assert!(should_reload(false, None, ttl));
    assert!(!should_reload(false, Some(Duration::from_secs(5)), ttl));
    assert!(should_reload(false, Some(Duration::from_secs(31)), ttl));
}

#[test]
fn suggestion_storage_limit_is_pinned_around_boundary() {
    for count in [31, 32, 33] {
        let files = workspace_files(count);
        let expected = fuzzy_filter(&files, "file", |s| s.as_str())
            .into_iter()
            .take(32)
            .cloned()
            .collect::<Vec<_>>();
        let mut f = FilesAutocomplete::with_files(files, 8);

        f.update("@file", 5);

        assert_eq!(f.suggestion_count(), count.min(32));
        assert_eq!(f.suggestion_values(), expected);
    }
}

fn workspace_files(file_count: usize) -> Vec<String> {
    (0..file_count)
        .map(|i| format!("src/module_{i:04}/file_{i:04}.rs"))
        .collect()
}
