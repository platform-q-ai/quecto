//! Steps for `tui_list_render_state.feature` (#997).
//!
//! Characterization scenarios drive the REAL components (through the render
//! harness for the slash dropdown, and through world-held component instances
//! for the file popup / model selector, mirroring `tui_file_mention_steps`).
//! The shared-renderer scenarios compare each component's actual rendered rows
//! against `list_rows::render_list_rows` — RED until the helper exists. The
//! grouped-state scenarios drive real App paths through the harness and
//! observe the value through the owner-group probes.

use std::time::{Duration, Instant};

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::interface::app::tui_harness::{TuiHarness, subagent, subagents_changed};
use quecto_tui::interface::component::Component;
use quecto_tui::interface::components::autocomplete::{
    Autocomplete, AutocompleteResult, SlashCommand,
};
use quecto_tui::interface::components::files_autocomplete::FilesAutocomplete;
use quecto_tui::interface::components::list_navigator::ListNavigator;
use quecto_tui::interface::components::list_rows::{
    DescriptionMode, ListRow, RowStyle, render_list_rows, visible_window,
};
use quecto_tui::interface::components::model_selector::{ModelEntry, ModelSelector};
use quecto_tui::interface::components::select_list::{SelectItem, SelectList};
use quecto_tui::interface::keys::Key;

const ACCENT: &str = "\x1b[36m";
const DIM: &str = "\x1b[2m";

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut esc = false;
    for ch in s.chars() {
        if esc {
            if ch.is_ascii_alphabetic() || ch == '~' {
                esc = false;
            }
        } else if ch == '\x1b' {
            esc = true;
        } else {
            out.push(ch);
        }
    }
    out
}

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

#[then(regex = r#"^the slash dropdown windows the commands with the indicator "([^"]*)"$"#)]
fn slash_dropdown_windowed(world: &mut TuiWorld, indicator: String) {
    let (count, frame) = with_harness(world, |h| {
        (h.autocomplete_suggestion_count(), h.full_frame())
    });
    let plain = strip_ansi(&frame);
    assert_eq!(count, 12, "all built-in commands should be suggested");
    assert!(
        plain.contains(&indicator),
        "the composed frame must contain the overflow indicator {indicator}:\n{plain}"
    );
    assert!(
        plain.contains("→ /clear"),
        "the first command row carries the selection arrow:\n{plain}"
    );
    assert!(
        !plain.contains("/workflow-nudge"),
        "rows beyond the 8-row window must not be drawn:\n{plain}"
    );
}

// ── Files popup characterization (world-held real component) ────────────────

#[given("a shared-list files popup loaded with a stale workspace file list")]
fn files_popup_stale(world: &mut TuiWorld) {
    let mut f =
        FilesAutocomplete::with_files(vec!["src/main.rs".to_string(), "src/lib.rs".to_string()], 5);
    f.mark_loaded_at_for_test(Instant::now() - Duration::from_secs(31));
    world.tui_files_autocomplete = Some(f);
}

#[given("a shared-list files popup with no loaded files")]
fn files_popup_fresh(world: &mut TuiWorld) {
    world.tui_files_autocomplete = Some(FilesAutocomplete::new(5));
}

#[when("the shared-list files popup is opened with an at token")]
fn files_popup_open(world: &mut TuiWorld) {
    let f = world.tui_files_autocomplete.as_mut().expect("files popup");
    f.update("@", 1);
    assert!(f.is_active(), "an @ token must activate the popup");
}

#[then("a shared-list background reload is requested")]
fn files_reload_requested(world: &mut TuiWorld) {
    let f = world.tui_files_autocomplete.as_mut().expect("files popup");
    world.tui_files_load_requested = f.take_load_request();
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

#[then("accepting the placeholder leaves the file result pending")]
fn files_placeholder_not_accepted(world: &mut TuiWorld) {
    let f = world.tui_files_autocomplete.as_mut().expect("files popup");
    f.handle_input(&Key::Tab);
    assert_eq!(
        f.take_result(),
        AutocompleteResult::Pending,
        "Tab must not accept the loading placeholder"
    );
    assert!(f.is_active(), "the popup stays open while loading");
}

// ── Model selector characterization (world-held real component) ─────────────

fn marker_fixture() -> ModelSelector {
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

#[when(regex = r#"^the model selection moves down (\d+) rows$"#)]
fn model_selection_down(world: &mut TuiWorld, n: usize) {
    let sel = &mut world.tui_list_model_selector.as_mut().expect("selector").0;
    for _ in 0..n {
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
    let sel = &mut world.tui_list_model_selector.as_mut().expect("selector").0;
    let lines = sel.render(60);
    let marked = lines
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
    let sel = &mut world.tui_list_model_selector.as_mut().expect("selector").0;
    let lines: Vec<String> = sel.render(60).iter().map(|l| strip_ansi(l)).collect();
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

// ── Shared row helper equivalence (RED until #997 lands) ────────────────────

#[given("the four list surfaces hold sample rows")]
fn four_surfaces_sample(world: &mut TuiWorld) {
    let items: Vec<SelectItem> = [
        ("alpha", Some("first")),
        ("beta", Some("second")),
        ("gamma-long", None),
        ("delta", None),
        ("epsilon", None),
    ]
    .iter()
    .map(|(label, desc)| SelectItem {
        value: label.to_string(),
        label: label.to_string(),
        description: desc.map(str::to_string),
    })
    .collect();
    world.tui_list_select = Some(crate::DebugSelectList(SelectList::new(items, 3)));

    let commands = vec![
        SlashCommand {
            name: "model".into(),
            description: "Select model".into(),
        },
        SlashCommand {
            name: "clear".into(),
            description: "Clear history".into(),
        },
        SlashCommand {
            name: "quit".into(),
            description: "Exit TUI".into(),
        },
    ];
    let mut ac = Autocomplete::new(commands, 2);
    ac.update("/");
    world.tui_list_autocomplete = Some(crate::DebugAutocomplete(ac));

    let mut files =
        FilesAutocomplete::with_files(vec!["src/main.rs".to_string(), "src/lib.rs".to_string()], 5);
    files.update("@", 1);
    world.tui_files_autocomplete = Some(files);

    world.tui_list_model_selector = Some(crate::DebugModelSelector(marker_fixture()));
}

#[then("the shared row helper reproduces the select list rows exactly")]
fn helper_matches_select_list(world: &mut TuiWorld) {
    let list = &mut world.tui_list_select.as_mut().expect("select list").0;
    let expected = list.render(60);
    let labels_descs: Vec<(String, Option<String>)> = [
        ("alpha", Some("first")),
        ("beta", Some("second")),
        ("gamma-long", None),
        ("delta", None),
        ("epsilon", None),
    ]
    .iter()
    .map(|(l, d)| (l.to_string(), d.map(str::to_string)))
    .collect();
    let nav = ListNavigator::new();
    let window = visible_window(&nav, labels_descs.len(), 3);
    let rows: Vec<ListRow> = window
        .clone()
        .map(|i| {
            let mut row = ListRow::plain(labels_descs[i].0.clone());
            row.description = labels_descs[i].1.clone();
            row
        })
        .collect();
    let style = RowStyle {
        indent: "",
        description: DescriptionMode::AlignedWindow { min_desc_width: 10 },
    };
    let got = render_list_rows(&rows, &nav, labels_descs.len(), 3, 60, &style);
    assert_eq!(got, expected, "shared helper must reproduce SelectList");
}

#[then("the shared row helper reproduces the slash dropdown rows exactly")]
fn helper_matches_autocomplete(world: &mut TuiWorld) {
    let ac = &mut world.tui_list_autocomplete.as_mut().expect("dropdown").0;
    let expected = ac.render(60);
    let data = [
        ("/model", "Select model"),
        ("/clear", "Clear history"),
        ("/quit", "Exit TUI"),
    ];
    let nav = ListNavigator::new();
    let window = visible_window(&nav, data.len(), 2);
    let rows: Vec<ListRow> = window
        .clone()
        .map(|i| {
            let mut row = ListRow::plain(data[i].0);
            row.description = Some(data[i].1.to_string());
            row
        })
        .collect();
    let style = RowStyle {
        indent: "",
        description: DescriptionMode::Inline,
    };
    let got = render_list_rows(&rows, &nav, data.len(), 2, 60, &style);
    assert_eq!(got, expected, "shared helper must reproduce Autocomplete");
}

#[then("the shared row helper reproduces the files dropdown rows exactly")]
fn helper_matches_files(world: &mut TuiWorld) {
    let f = world.tui_files_autocomplete.as_mut().expect("files popup");
    let expected = f.render(60);
    let paths = ["@src/main.rs", "@src/lib.rs"];
    let nav = ListNavigator::new();
    let rows: Vec<ListRow> = paths.iter().map(|p| ListRow::plain(*p)).collect();
    let style = RowStyle {
        indent: "",
        description: DescriptionMode::Inline,
    };
    let got = render_list_rows(&rows, &nav, paths.len(), 5, 60, &style);
    assert_eq!(
        got, expected,
        "shared helper must reproduce FilesAutocomplete"
    );
}

#[then("the shared row helper reproduces the model selector rows exactly")]
fn helper_matches_model_selector(world: &mut TuiWorld) {
    let sel = &mut world.tui_list_model_selector.as_mut().expect("selector").0;
    // Rows only: skip the title / search / spacer header lines.
    let expected: Vec<String> = sel.render(60).split_off(3);
    let data = [("a-model", "ProvA", ""), ("model-bb-long", "ProvB", " ●")];
    let nav = ListNavigator::new();
    let rows: Vec<ListRow> = data
        .iter()
        .map(|(id, provider, marker)| {
            let mut row = ListRow::plain(*id);
            row.description = Some(provider.to_string());
            row.marker = marker;
            row
        })
        .collect();
    let style = RowStyle {
        indent: "  ",
        description: DescriptionMode::AlignedCached { label_width: 13 },
    };
    let got = render_list_rows(&rows, &nav, data.len(), 12, 60, &style);
    assert_eq!(
        got, expected,
        "shared helper must reproduce ModelSelector rows"
    );
}

// ── Grouped App state (RED until the owner structs exist) ───────────────────

#[given("a live TUI render harness")]
fn live_harness(world: &mut TuiWorld) {
    with_harness(world, |_h| {});
}

#[when("a rewind open request is issued by double Escape")]
fn rewind_open_issued(world: &mut TuiWorld) {
    with_harness(world, |h| h.issue_rewind_open());
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
    assert!(
        cmds.iter().any(|c| c.contains("rewind-open-1")),
        "the real double-Escape path must issue rewind-open-1: {cmds:?}"
    );
}

#[then(regex = r#"^the rewind owner group reports request sequence (\d+)$"#)]
fn rewind_group_seq(world: &mut TuiWorld, seq: u64) {
    let got = with_harness(world, |h| h.rewind_group_request_seq());
    assert_eq!(got, seq, "the rewind owner group must hold the issued seq");
}

#[when(regex = r#"^a list_models response with (\d+) models arrives$"#)]
fn list_models_arrives(world: &mut TuiWorld, n: usize) {
    with_harness(world, |h| h.deliver_list_models(n));
}

#[then(regex = r#"^the model registry group holds (\d+) entries with no pending open$"#)]
fn model_registry_group(world: &mut TuiWorld, n: usize) {
    let (entries, pending) = with_harness(world, |h| {
        (
            h.model_registry_group_entries(),
            h.model_registry_group_pending(),
        )
    });
    assert_eq!(entries, n, "registry group must hold the parsed entries");
    assert!(!pending, "an unsolicited response leaves no pending open");
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
