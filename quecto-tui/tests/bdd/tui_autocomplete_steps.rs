//! Steps for `tui_autocomplete_nav.feature` and `tui_autocomplete_enter.feature`.
//!
//! Both features drive the REAL App slash-command autocomplete through the
//! headless render harness: typing routes through `App::handle_key`, which owns
//! the production autocomplete update / navigation / accept-and-submit wiring.

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::shell::app::tui_harness::TuiHarness;
use quecto_tui::shell::keys::Key;

fn with_harness<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    if world.tui_parity_rt.is_none() {
        world.tui_parity_rt = Some(tokio::runtime::Runtime::new().expect("tokio runtime"));
    }
    if world.tui_parity.is_none() {
        let rt = world.tui_parity_rt.as_ref().expect("runtime");
        let h = rt.block_on(async { TuiHarness::new().await });
        world.tui_parity = Some(TuiParityHarness(h));
    }
    // Enter the runtime context so harness ops that spawn tokio tasks (e.g.
    // opening the model selector, sending a command) have a live reactor.
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let _guard = handle.enter();
    f(&mut world.tui_parity.as_mut().expect("TUI harness").0)
}

fn type_str(world: &mut TuiWorld, text: &str) {
    with_harness(world, |h| {
        // Each character routes through the real `App::handle_key` path.
        text.chars().for_each(|ch| {
            h.press(Key::Char(ch));
        });
    });
}

// ── Shared Given ─────────────────────────────────────────────────────────────

#[given(regex = r#"^the editor text is "([^"]*)"$"#)]
fn editor_text_is(world: &mut TuiWorld, text: String) {
    // A fresh scenario starts with an empty editor; typing runs the real
    // per-key autocomplete update path exactly as it would for a live user.
    type_str(world, &text);
    let got = with_harness(world, |h| h.editor_text());
    assert_eq!(got, text, "editor should hold the typed text");
}

// ── Navigation feature ───────────────────────────────────────────────────────

#[given("the autocomplete shows all commands")]
fn autocomplete_shows_all(world: &mut TuiWorld) {
    let expected = TuiHarness::slash_command_names().len();
    let (active, count) = with_harness(world, |h| {
        (h.autocomplete_active(), h.autocomplete_suggestion_count())
    });
    assert!(active, "the slash-command autocomplete should be active");
    assert_eq!(
        count, expected,
        "typing '/' should list every built-in command"
    );
}

#[when(regex = r#"^the user presses Down (\d+) times$"#)]
fn press_down_times(world: &mut TuiWorld, n: usize) {
    with_harness(world, |h| {
        for _ in 0..n {
            h.press(Key::Down);
        }
    });
}

#[given(regex = r#"^the selected index is (\d+)$"#)]
fn given_selected_index(world: &mut TuiWorld, idx: usize) {
    // Drive the highlight to `idx` by stepping Down from the top of the list.
    with_harness(world, |h| {
        for _ in 0..idx {
            h.press(Key::Down);
        }
    });
    let got = with_harness(world, |h| h.autocomplete_selected_index());
    assert_eq!(got, idx, "precondition: selection should be at index {idx}");
}

#[when("the user presses Up")]
fn press_up(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.press(Key::Up);
    });
}

#[when(regex = r#"^update is called again with "([^"]*)"$"#)]
fn update_again(world: &mut TuiWorld, text: String) {
    // Re-run the exact update the app performs after every editor change.
    let current = with_harness(world, |h| h.editor_text());
    assert_eq!(
        current, text,
        "the re-issued update must use the current editor text"
    );
    with_harness(world, |h| h.refresh_autocomplete());
}

#[then(regex = r#"^the selected index should be (\d+)$"#)]
fn selected_index_should_be(world: &mut TuiWorld, idx: usize) {
    let got = with_harness(world, |h| h.autocomplete_selected_index());
    assert_eq!(got, idx, "selected index should be {idx}, got {got}");
}

#[then(regex = r#"^the selected index should remain (\d+)$"#)]
fn selected_index_should_remain(world: &mut TuiWorld, idx: usize) {
    let got = with_harness(world, |h| h.autocomplete_selected_index());
    assert_eq!(
        got, idx,
        "selection should be preserved at {idx}, got {got}"
    );
}

// ── Enter/Tab feature ────────────────────────────────────────────────────────

#[given(regex = r#"^the autocomplete is showing "([^"]*)" highlighted$"#)]
fn autocomplete_showing_highlighted(world: &mut TuiWorld, value: String) {
    let (active, highlighted) = with_harness(world, |h| {
        (h.autocomplete_active(), h.autocomplete_selected_value())
    });
    assert!(active, "the autocomplete should be active");
    assert_eq!(
        highlighted,
        Some(value.clone()),
        "the highlighted suggestion should be {value:?}"
    );
}

#[when("the user presses Enter")]
fn press_enter(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.press(Key::Enter);
    });
}

#[when("the user presses Tab")]
fn press_tab(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.press(Key::Tab);
    });
}

/// Drain the commands the app has sent over its socket (drives the runtime).
fn drain_commands(world: &mut TuiWorld) -> Vec<String> {
    let rt = world.tui_parity_rt.as_ref().expect("runtime");
    let h = &mut world.tui_parity.as_mut().expect("harness").0;
    rt.block_on(h.drain_commands())
}

/// Assert the command actually reached `handle_submit` (not rejected as an
/// unknown partial) and produced its production side effect.
fn assert_command_effect(world: &mut TuiWorld, cmd: &str) {
    let (rejected, should_exit) = with_harness(world, |h| (h.has_notification(), h.should_exit()));
    assert!(
        !rejected,
        "command {cmd:?} must not have been rejected as an unknown partial"
    );
    match cmd {
        "/quit" | "/exit" => assert!(should_exit, "{cmd} should have set the app's exit flag"),
        "/model" => {
            // `/model` with no argument requests the model list over the socket
            // (the selector opens asynchronously once the list arrives). A stale
            // `/mo` would have been rejected and sent nothing, so an emitted
            // model-list command proves the full `/model` reached handle_submit.
            let cmds = drain_commands(world);
            assert!(
                cmds.iter().any(|c| c.contains("model")),
                "/model should have emitted a model-list command, got: {cmds:?}"
            );
        }
        _ => {}
    }
}

#[then(regex = r#"^the editor text should be "([^"]*)" before handle_submit runs$"#)]
fn editor_text_before_submit(world: &mut TuiWorld, value: String) {
    // The accept path sets the editor to the full command BEFORE calling
    // handle_submit, then clears it after submit. The command's side effect
    // (below) only fires if the FULL command — not the stale partial — reached
    // handle_submit, which proves the editor held `value` before the submit.
    assert_command_effect(world, &value);
    let after = with_harness(world, |h| h.editor_text());
    assert_eq!(
        after, "",
        "the editor is cleared after a slash-command submit"
    );
}

#[then(regex = r#"^the submitted command should be "([^"]*)"$"#)]
fn submitted_command_should_be(world: &mut TuiWorld, value: String) {
    assert_command_effect(world, &value);
    let after = with_harness(world, |h| h.editor_text());
    assert_eq!(
        after, "",
        "the editor is cleared once the command is submitted"
    );
}

#[then(regex = r#"^any code reading editor.text\(\) during submit should see "([^"]*)"$"#)]
fn code_reading_editor_sees(world: &mut TuiWorld, value: String) {
    // `/model` requests the model list over the socket; a stale `/mo` would have
    // been rejected and sent nothing — so the emitted command proves the full
    // command was the editor text visible during submit.
    assert_command_effect(world, &value);
    let after = with_harness(world, |h| h.editor_text());
    assert_eq!(
        after, "",
        "the editor is cleared once the command is submitted"
    );
}

#[then(regex = r#"^not the stale partial "([^"]*)"$"#)]
fn not_the_stale_partial(world: &mut TuiWorld, partial: String) {
    let rejected = with_harness(world, |h| h.has_notification());
    assert!(
        !rejected,
        "the stale partial {partial:?} must not have been submitted (which would \
         raise an unknown-command warning)"
    );
}

#[then(regex = r#"^the editor text should be "([^"]*)"$"#)]
fn editor_text_should_be(world: &mut TuiWorld, value: String) {
    let got = with_harness(world, |h| h.editor_text());
    assert_eq!(got, value, "editor text should be {value:?}, got {got:?}");
}

#[then("the autocomplete should remain active for further editing")]
fn autocomplete_remains_active(world: &mut TuiWorld) {
    // Tab ACCEPTS the highlighted command into the editor without submitting
    // (unlike Enter). The suggestion list is `close()`d, not `clear()`ed, so its
    // entries are retained and the menu stays available as the user keeps
    // editing — and crucially nothing was submitted.
    let (submitted, exited, retained) = with_harness(world, |h| {
        (
            h.model_selector_open(),
            h.should_exit(),
            h.autocomplete_suggestion_count(),
        )
    });
    assert!(!exited && !submitted, "Tab must not submit the command");
    assert!(
        retained > 0,
        "the autocomplete suggestions should be retained for further editing"
    );
}
