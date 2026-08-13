//! Steps for `tui_list_render_state.feature` (#997).
//!
//! Characterization scenarios drive the REAL components (through the render
//! harness for the slash dropdown, and through world-held component instances
//! for the file popup / model selector, mirroring `tui_file_mention_steps`).
//! The grouped-state scenarios drive real App paths through the harness and
//! observe the value through the owner-group probes. Helper-vs-component
//! render equivalence lives in the `list_rows` unit tests and the pixel
//! characterization tests, not here.

use std::time::{Duration, Instant};

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::components::autocomplete::AutocompleteResult;
use quecto_tui::components::component::Component;
use quecto_tui::components::files_autocomplete::FilesAutocomplete;
use quecto_tui::components::model_selector::ModelSelector;
use quecto_tui::shell::app::tui_harness::{TuiHarness, subagent, subagents_changed};
use quecto_tui::shell::keys::Key;

const ACCENT: &str = "\x1b[36m";
const DIM: &str = "\x1b[2m";

use quecto_tui::components::ansi::strip_ansi;

fn with_harness<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    if world.tui_parity_rt.is_none() {
        world.tui_parity_rt = Some(tokio::runtime::Runtime::new().expect("tokio runtime"));
    }
    if world.tui_parity.is_none() {
        let rt = world.tui_parity_rt.as_ref().expect("runtime");
        let h = rt.block_on(async { TuiHarness::new().await });
        world.tui_parity = Some(TuiParityHarness(h));
    }
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let _guard = handle.enter();
    f(&mut world.tui_parity.as_mut().expect("TUI harness").0)
}

// ── Slash dropdown characterization (through the real App render path) ──────

#[when("the interface renders a frame")]
fn interface_renders_frame(world: &mut TuiWorld) {
    world.stdout = with_harness(world, |h| h.full_frame());
}

#[then(
    regex = r#"^the slash dropdown draws exactly the first 8 commands with the indicator "([^"]*)"$"#
)]
fn slash_dropdown_windowed(world: &mut TuiWorld, indicator: String) {
    let count = with_harness(world, |h| h.autocomplete_suggestion_count());
    let plain = strip_ansi(&world.stdout);
    let names = TuiHarness::slash_command_names();
    assert_eq!(
        count,
        names.len(),
        "all built-in commands should be suggested"
    );
    assert_eq!(count, 15, "the built-in command set is 15 commands");
    // Positive windowing lock: a drawn row is `/{name}` followed by the fixed
    // two-space description gap. Exactly the first 8 commands are drawn.
    let drawn: Vec<String> = names
        .iter()
        .filter(|n| plain.contains(&format!("/{n}  ")))
        .cloned()
        .collect();
    assert_eq!(
        drawn,
        names[..8].to_vec(),
        "exactly the first 8 command rows must be drawn:\n{plain}"
    );
    assert!(
        plain.contains(&indicator),
        "the composed frame must contain the overflow indicator {indicator}:\n{plain}"
    );
    assert!(
        plain.contains("→ /clear"),
        "the first command row carries the selection arrow:\n{plain}"
    );
}

// ── Files popup characterization (world-held real component) ────────────────

#[given("a files popup loaded with a stale workspace file list")]
fn files_popup_stale(world: &mut TuiWorld) {
    let mut f =
        FilesAutocomplete::with_files(vec!["src/main.rs".to_string(), "src/lib.rs".to_string()], 5);
    f.mark_loaded_at_for_test(Instant::now() - Duration::from_secs(31));
    world.tui_files_autocomplete = Some(f);
}

#[given("a files popup with no loaded files")]
fn files_popup_fresh(world: &mut TuiWorld) {
    world.tui_files_autocomplete = Some(FilesAutocomplete::new(5));
}

#[given("a files popup showing the loading placeholder")]
fn files_popup_loading_placeholder(world: &mut TuiWorld) {
    let mut f = FilesAutocomplete::new(5);
    f.update("@", 1);
    world.tui_files_load_requested = f.take_load_request();
    world.tui_files_autocomplete = Some(f);
}

#[when("the user types an at token")]
fn files_popup_open(world: &mut TuiWorld) {
    let f = world.tui_files_autocomplete.as_mut().expect("files popup");
    f.update("@", 1);
    // Consume the latch here (the action side) so the Then only reads it.
    world.tui_files_load_requested = f.take_load_request();
}

#[when("the user accepts the highlighted row")]
fn files_accept_highlighted(world: &mut TuiWorld) {
    let f = world.tui_files_autocomplete.as_mut().expect("files popup");
    f.handle_input(&Key::Tab);
}

#[then("a background reload is requested")]
fn files_reload_requested(world: &mut TuiWorld) {
    assert!(
        world.tui_files_load_requested,
        "a stale list must latch a background load request"
    );
}

#[then("the selected file row keeps its arrow marker undimmed")]
fn files_selected_row_marker(world: &mut TuiWorld) {
    let f = world.tui_files_autocomplete.as_mut().expect("files popup");
    let lines = f.render(60);
    assert_eq!(strip_ansi(&lines[0]), "→ @src/main.rs");
    assert!(
        lines[0].contains(ACCENT),
        "the selected row keeps its accent while a reload is pending: {:?}",
        lines[0]
    );
    assert!(
        !lines[0].contains(DIM),
        "real file rows are never dimmed by a background reload: {:?}",
        lines[0]
    );
}

#[then("the only file row is a dimmed loading placeholder")]
fn files_loading_placeholder(world: &mut TuiWorld) {
    let f = world.tui_files_autocomplete.as_mut().expect("files popup");
    let lines = f.render(60);
    assert_eq!(lines.len(), 1);
    assert_eq!(strip_ansi(&lines[0]), "→ loading files…");
    assert!(lines[0].contains(DIM), "placeholder must be dim");
    assert!(
        !lines[0].contains(ACCENT),
        "placeholder must never be accented"
    );
}

#[then("no file is inserted and the popup stays open")]
fn files_placeholder_not_accepted(world: &mut TuiWorld) {
    let f = world.tui_files_autocomplete.as_mut().expect("files popup");
    assert_eq!(
        f.take_result(),
        AutocompleteResult::Pending,
        "accepting must not select the loading placeholder"
    );
    assert!(f.is_active(), "the popup stays open while loading");
}

// ── Model selector characterization (world-held real component) ─────────────

fn marker_fixture() -> ModelSelector {
    use quecto_tui::components::model_selector::ModelEntry;
    let models = vec![
        ModelEntry {
            id: "a-model".to_string(),
            provider: "ProvA".to_string(),
            auth: None,
            is_current: false,
        },
        ModelEntry {
            id: "model-bb-long".to_string(),
            provider: "ProvB".to_string(),
            auth: None,
            is_current: false,
        },
    ];
    ModelSelector::with_models(models, Some("model-bb-long"))
}

#[given("a model selector over the known models")]
fn model_selector_known(world: &mut TuiWorld) {
    world.tui_list_model_selector = Some(crate::DebugModelSelector(ModelSelector::new(None)));
}

#[given("a model selector whose current model has the longest id")]
fn model_selector_longest_current(world: &mut TuiWorld) {
    world.tui_list_model_selector = Some(crate::DebugModelSelector(marker_fixture()));
}

#[given(regex = r#"^the model selection rests on the (\d+)(?:st|nd|rd|th) model$"#)]
fn model_selection_rests_on(world: &mut TuiWorld, nth: usize) {
    let sel = &mut world.tui_list_model_selector.as_mut().expect("selector").0;
    for _ in 1..nth {
        sel.handle_input(&Key::Down);
    }
}

#[when(regex = r#"^the model filter "([^"]*)" is typed$"#)]
fn model_filter_typed(world: &mut TuiWorld, filter: String) {
    let sel = &mut world.tui_list_model_selector.as_mut().expect("selector").0;
    for c in filter.chars() {
        sel.handle_input(&Key::Char(c));
    }
}

#[when("the model selector renders")]
fn model_selector_renders(world: &mut TuiWorld) {
    let sel = &mut world.tui_list_model_selector.as_mut().expect("selector").0;
    world.tui_list_rendered = sel.render(60);
}

#[then(regex = r#"^(\d+) models match and the selection is clamped to the last match$"#)]
fn model_selection_clamped(world: &mut TuiWorld, n: usize) {
    let sel = &mut world.tui_list_model_selector.as_mut().expect("selector").0;
    assert_eq!(sel.visible_count(), n, "filter should leave {n} matches");
    let selected = sel.selected_model().expect("clamped selection").id.clone();
    assert!(
        selected.contains("kimi"),
        "selection must CLAMP to the last match (kimi), not reset to row 0: {selected}"
    );
}

#[then("the current model row carries the marker after its id")]
fn model_marker_present(world: &mut TuiWorld) {
    let marked = world
        .tui_list_rendered
        .iter()
        .map(|l| strip_ansi(l))
        .find(|l| l.contains('●'))
        .expect("a row must carry the current-model marker");
    assert!(
        marked.contains("model-bb-long ●"),
        "the marker follows the current model's id: {marked:?}"
    );
}

#[then("the marked row's provider is offset by exactly the marker width")]
fn model_marker_offset(world: &mut TuiWorld) {
    let lines: Vec<String> = world
        .tui_list_rendered
        .iter()
        .map(|l| strip_ansi(l))
        .collect();
    let unmarked_col = lines
        .iter()
        .find_map(|l| l.find("ProvA"))
        .expect("unmarked row present");
    let marked_col = lines
        .iter()
        .find_map(|l| l.find("ProvB"))
        .expect("marked row present");
    // Today's pixels: the ` ●` marker sits outside the alignment column, so
    // the marked row's provider starts exactly 2 cells later. Any other diff
    // is an alignment regression.
    assert_eq!(
        marked_col,
        unmarked_col + 2,
        "marker must shift the provider by its own width only:\n{lines:?}"
    );
}

// ── Grouped App state (observed through the owner-group probes) ─────────────

#[given("a live TUI render harness")]
fn live_harness(world: &mut TuiWorld) {
    with_harness(world, |_h| {});
}

#[when("a rewind open request is issued by double Escape")]
fn rewind_open_issued(world: &mut TuiWorld) {
    with_harness(world, |h| h.issue_rewind_open());
}

#[then("a rewind-open command is emitted")]
fn rewind_open_emitted(world: &mut TuiWorld) {
    let cmds = {
        let handle = world
            .tui_parity_rt
            .as_ref()
            .expect("runtime")
            .handle()
            .clone();
        let harness = &mut world.tui_parity.as_mut().expect("harness").0;
        handle.block_on(harness.drain_commands())
    };
    // The id is `rewind-open-<uuid_like>-<request_seq>` (#1314): match the
    // prefix and the seq-1 suffix, not the process-unique middle — the old
    // `contains("rewind-open-1")` only matched when the embedded hex PID
    // happened to start with a 1.
    assert!(
        cmds.iter().any(|c| c.contains(r#""id":"tab0:rewind-open-"#)
            && c.contains(r#""type":"get_messages""#)
            && c.contains(r#"-1""#)),
        "the real double-Escape path must issue a seq-1 rewind-open get_messages: {cmds:?}"
    );
}

#[then(regex = r#"^the rewind owner group reports request sequence (\d+)$"#)]
fn rewind_group_seq(world: &mut TuiWorld, seq: u64) {
    let got = with_harness(world, |h| h.rewind_group_request_seq());
    assert_eq!(got, seq, "the rewind owner group must hold the issued seq");
}

#[given("a model selector open has been requested")]
fn model_selector_open_requested(world: &mut TuiWorld) {
    world.tui_model_open_was_pending = with_harness(world, |h| {
        h.request_model_selector_open();
        h.model_registry_group().1
    });
}

#[when(regex = r#"^a list_models response with (\d+) models arrives$"#)]
fn list_models_arrives(world: &mut TuiWorld, n: usize) {
    with_harness(world, |h| h.deliver_list_models(n));
}

#[then(regex = r#"^the model registry group holds (\d+) entries and the pending open is cleared$"#)]
fn model_registry_group(world: &mut TuiWorld, n: usize) {
    let (entries, pending) = with_harness(world, |h| h.model_registry_group());
    assert!(
        world.tui_model_open_was_pending,
        "the selector-open request must set the pending flag first"
    );
    assert_eq!(entries, n, "registry group must hold the parsed entries");
    assert!(!pending, "the delivered response clears the pending open");
}

#[when(regex = r#"^a subagents_changed push registers (\d+) agent$"#)]
fn subagents_push(world: &mut TuiWorld, n: usize) {
    with_harness(world, |h| {
        let infos = (0..n)
            .map(|i| subagent(&format!("bdd-997-a{i}"), "running", Some(("active", 0, 3))))
            .collect();
        h.event(subagents_changed(infos));
    });
}

#[then(regex = r#"^the sub-agent owner group tracks (\d+) agent$"#)]
fn subagent_group_tracks(world: &mut TuiWorld, n: usize) {
    let got = with_harness(world, |h| h.subagent_group_tracked());
    assert_eq!(got, n, "the sub-agent UI owner group must track the push");
}
