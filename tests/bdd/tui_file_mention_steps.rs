//! Steps for `tui_file_mention.feature` — drive the quecto-tui
//! `FilesAutocomplete` component directly with an injected file list.

use crate::QuectoWorld;
use cucumber::{given, then, when};
use quecto_tui::interface::component::Component;
use quecto_tui::interface::components::autocomplete::AutocompleteResult;
use quecto_tui::interface::components::files_autocomplete::FilesAutocomplete;
use quecto_tui::interface::keys::Key;

#[given(regex = r#"^a workspace with files "([^"]*)"$"#)]
fn a_workspace_with_files(world: &mut QuectoWorld, csv: String) {
    let files: Vec<String> = csv
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    world.tui_files_autocomplete = Some(FilesAutocomplete::with_files(files, 8));
}

#[when(regex = r#"^the user types "([^"]*)" in the editor$"#)]
fn the_user_types(world: &mut QuectoWorld, text: String) {
    let fa = world
        .tui_files_autocomplete
        .as_mut()
        .expect("workspace files must be set first");
    // Cursor sits at the end of the typed text.
    fa.update(&text, text.len());
}

#[then("the file mention popup is active")]
fn popup_active(world: &mut QuectoWorld) {
    assert!(
        world.tui_files_autocomplete.as_ref().unwrap().is_active(),
        "expected the file mention popup to be active"
    );
}

#[then("the file mention popup is not active")]
fn popup_not_active(world: &mut QuectoWorld) {
    assert!(
        !world.tui_files_autocomplete.as_ref().unwrap().is_active(),
        "expected the file mention popup to be inactive"
    );
}

#[then(regex = r#"^the file mention popup lists "([^"]*)"$"#)]
fn popup_lists(world: &mut QuectoWorld, needle: String) {
    let fa = world.tui_files_autocomplete.as_mut().unwrap();
    let rendered = fa.render(80).join("\n");
    let plain: String = rendered
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect();
    assert!(plain.contains(&needle), "expected {needle:?} in: {plain}");
}

#[when("the user accepts the file mention")]
fn accept_file_mention(world: &mut QuectoWorld) {
    world
        .tui_files_autocomplete
        .as_mut()
        .unwrap()
        .handle_input(&Key::Tab);
}

#[then(regex = r#"^the selected file is "([^"]*)"$"#)]
fn selected_file_is(world: &mut QuectoWorld, expected: String) {
    let fa = world.tui_files_autocomplete.as_mut().unwrap();
    match fa.take_result() {
        AutocompleteResult::Selected(v) => assert_eq!(v, expected),
        other => panic!("expected Selected({expected:?}), got {other:?}"),
    }
}
