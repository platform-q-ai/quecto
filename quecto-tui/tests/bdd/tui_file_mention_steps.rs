//! Steps for `tui_file_mention.feature` — drive the quecto-tui
//! `FilesAutocomplete` component directly with an injected file list.

use std::time::{Duration, Instant};

use crate::TuiWorld;
use cucumber::{given, then, when};
use quecto_tui::components::autocomplete::AutocompleteResult;
use quecto_tui::components::component::Component;
use quecto_tui::components::files_autocomplete::FilesAutocomplete;
use quecto_tui::infrastructure::workspace_files::MAX_WORKSPACE_FILES;
use quecto_tui::shell::keys::Key;

#[given(regex = r#"^a workspace with files "([^"]*)"$"#)]
fn a_workspace_with_files(world: &mut TuiWorld, csv: String) {
    let files = parse_files(&csv);
    world.tui_files_autocomplete = Some(FilesAutocomplete::with_files(files, 8));
    world.tui_files_load_requested = false;
}

#[given("workspace file loading is pending")]
fn workspace_file_loading_is_pending(world: &mut TuiWorld) {
    world.tui_files_autocomplete = Some(FilesAutocomplete::new(8));
    world.tui_files_load_requested = false;
}

#[given("generated workspace file lists below and above the file mention limit")]
fn generated_workspace_file_lists_below_and_above_limit(world: &mut TuiWorld) {
    world.tui_files_filter_suggestion_counts.clear();
}

#[when(regex = r#"^each generated file list is filtered with the file mention "([^"]*)"$"#)]
fn each_generated_file_list_is_filtered(world: &mut TuiWorld, text: String) {
    world.tui_files_filter_suggestion_counts.clear();
    for file_count in [
        MAX_WORKSPACE_FILES - 1,
        MAX_WORKSPACE_FILES,
        MAX_WORKSPACE_FILES + 1,
    ] {
        let files = workspace_files(file_count);
        let mut fa = FilesAutocomplete::with_files(files, 8);
        fa.update("@", 1);
        fa.update(&text, text.len());
        world
            .tui_files_filter_suggestion_counts
            .push(fa.suggestion_count());
    }
}

#[when(regex = r#"^the user types "([^"]*)" in the editor$"#)]
fn the_user_types(world: &mut TuiWorld, text: String) {
    let fa = world
        .tui_files_autocomplete
        .as_mut()
        .expect("workspace files must be set first");
    // Cursor sits at the end of the typed text.
    fa.update(&text, text.len());
    world.tui_files_load_requested = fa.take_load_request();
}

#[given("workspace files were loaded more than 30 seconds ago")]
fn workspace_files_were_loaded_more_than_30_seconds_ago(world: &mut TuiWorld) {
    let mut fa = FilesAutocomplete::with_files(vec!["src/main.rs".to_string()], 8);
    fa.mark_loaded_at_for_test(Instant::now() - Duration::from_secs(31));
    world.tui_files_autocomplete = Some(fa);
    world.tui_files_load_requested = false;
}

#[when(regex = r#"^workspace files finish loading "([^"]*)"$"#)]
fn workspace_files_finish_loading(world: &mut TuiWorld, csv: String) {
    let files = parse_files(&csv);
    let fa = world.tui_files_autocomplete.as_mut().unwrap();
    fa.apply_loaded_files(files);
    // Re-run the same active token so loaded suggestions replace the loading
    // row, matching the app event loop after a background result arrives.
    fa.update("@", 1);
    world.tui_files_load_requested = fa.take_load_request();
}

#[then("the file mention popup is active")]
fn popup_active(world: &mut TuiWorld) {
    assert!(
        world.tui_files_autocomplete.as_ref().unwrap().is_active(),
        "expected the file mention popup to be active"
    );
}

#[then("the file mention popup is not active")]
fn popup_not_active(world: &mut TuiWorld) {
    assert!(
        !world.tui_files_autocomplete.as_ref().unwrap().is_active(),
        "expected the file mention popup to be inactive"
    );
}

#[then(regex = r#"^the file mention popup lists "([^"]*)"$"#)]
fn popup_lists(world: &mut TuiWorld, needle: String) {
    let fa = world.tui_files_autocomplete.as_mut().unwrap();
    let rendered = fa.render(80).join("\n");
    let plain: String = rendered
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    assert!(plain.contains(&needle), "expected {needle:?} in: {plain}");
}

#[when("the user accepts the file mention")]
fn accept_file_mention(world: &mut TuiWorld) {
    world
        .tui_files_autocomplete
        .as_mut()
        .unwrap()
        .handle_input(&Key::Tab);
}

#[then(regex = r#"^the selected file is "([^"]*)"$"#)]
fn selected_file_is(world: &mut TuiWorld, expected: String) {
    let fa = world.tui_files_autocomplete.as_mut().unwrap();
    match fa.take_result() {
        AutocompleteResult::Selected(v) => assert_eq!(v, expected),
        other => panic!("expected Selected({expected:?}), got {other:?}"),
    }
}

#[then("workspace file loading is requested")]
fn workspace_file_loading_is_requested(world: &mut TuiWorld) {
    assert!(
        world.tui_files_load_requested,
        "expected the component to request an async workspace-file load"
    );
}

#[then("filtering remains bounded as the generated workspace grows")]
fn filtering_remains_bounded_as_the_generated_workspace_grows(world: &mut TuiWorld) {
    assert_eq!(
        world.tui_files_filter_suggestion_counts.len(),
        3,
        "expected suggestion counts below, at, and above the file mention limit"
    );
    for &count in &world.tui_files_filter_suggestion_counts {
        assert_eq!(
            count, 32,
            "file mention filtering should keep only the bounded visible suggestion window"
        );
    }
}

#[then(regex = r#"^the file mention suggestions are exactly "([^"]*)"$"#)]
fn file_mention_suggestions_are_exactly(world: &mut TuiWorld, csv: String) {
    let expected = parse_files(&csv);
    let actual = world
        .tui_files_autocomplete
        .as_ref()
        .unwrap()
        .suggestion_values();
    assert_eq!(actual, expected);
}

fn parse_files(csv: &str) -> Vec<String> {
    csv.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn workspace_files(file_count: usize) -> Vec<String> {
    (0..file_count)
        .map(|i| format!("src/module_{i:04}/file_{i:04}.rs"))
        .collect()
}
