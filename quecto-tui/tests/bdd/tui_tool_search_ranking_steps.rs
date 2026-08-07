use cucumber::{given, then, when};
use quecto_tui::protocol::client::{ToolCatalogueEntry, ToolScope};
use quecto_tui::shell::keys::Key;

use crate::{TuiParityHarness, TuiWorld};

fn ensure_sized_harness(world: &mut TuiWorld) {
    if world.tui_parity_rt.is_none() {
        world.tui_parity_rt = Some(tokio::runtime::Runtime::new().expect("tokio runtime"));
    }
    let rt = world.tui_parity_rt.as_ref().expect("runtime");
    world.tui_parity = Some(TuiParityHarness(rt.block_on(async {
        quecto_tui::shell::app::tui_harness::TuiHarness::sized(120, 32).await
    })));
}

fn h(world: &mut TuiWorld) -> &mut quecto_tui::shell::app::tui_harness::TuiHarness {
    &mut world.tui_parity.as_mut().expect("harness").0
}

fn drain_commands(world: &mut TuiWorld) -> Vec<String> {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    handle.block_on(h(world).drain_commands())
}

fn catalogue_entry(name: &str, stable_id: &str, source: Option<&str>) -> ToolCatalogueEntry {
    ToolCatalogueEntry {
        stable_id: stable_id.into(),
        name: name.into(),
        source: source.map(str::to_string),
        profile_scope: Some(ToolScope::None),
        ..Default::default()
    }
}

fn open_management_with(world: &mut TuiWorld, entries: Vec<ToolCatalogueEntry>) {
    ensure_sized_harness(world);
    h(world).press(Key::Ctrl('t'));
    let _ = drain_commands(world);
    h(world).merge_tool_catalogue(entries);
}

fn plain_frame(world: &mut TuiWorld) -> String {
    quecto_tui::components::ansi::strip_ansi(&h(world).full_frame())
}

fn filtered_tool_lines(world: &mut TuiWorld) -> Vec<String> {
    plain_frame(world)
        .lines()
        .filter(|line| line.contains("[--]"))
        .map(str::to_string)
        .collect()
}

fn index_of_tool(lines: &[String], tool: &str) -> usize {
    lines
        .iter()
        .position(|line| line.contains(tool))
        .unwrap_or_else(|| panic!("missing {tool} in lines: {lines:#?}"))
}

#[given("the TUI has tools with ids, labels, aliases, and descriptions")]
fn tui_has_tools_with_searchable_fields(world: &mut TuiWorld) {
    open_management_with(
        world,
        vec![
            catalogue_entry("Describe", "describe", Some("metadata read assistance")),
            catalogue_entry("Read", "read", Some("filesystem access")),
            catalogue_entry("Notes", "notes", Some("read records from logs")),
            catalogue_entry(
                "Workflow Event Bridge",
                "workflow_event_bridge",
                Some("routes events"),
            ),
            catalogue_entry("Web Fetch", "web_fetch", Some("fetch a URL")),
            catalogue_entry("Fetch Web", "fetch_web", Some("network fetch")),
            catalogue_entry(
                "Remote",
                "remote",
                Some("web data from a cache fetch operation"),
            ),
            catalogue_entry(
                "Remote Shell Notes",
                "remote_shell",
                Some("documents command access"),
            ),
            catalogue_entry("Bash", "shell", Some("run commands")),
        ],
    );
}

#[when(expr = "the user filters tools by {string}")]
fn user_filters_tools_by(world: &mut TuiWorld, query: String) {
    for c in query.chars() {
        h(world).press(Key::Char(c));
    }
}

#[then(
    expr = "tools whose id or label contains {string} are ranked before tools that only mention {string} in the description"
)]
fn id_or_label_matches_precede_description_only(
    world: &mut TuiWorld,
    needle: String,
    description_needle: String,
) {
    let lines = filtered_tool_lines(world);
    assert!(
        needle == "read" && description_needle == "read",
        "unexpected fixture query: {needle}/{description_needle}"
    );
    let read = index_of_tool(&lines, "Read");
    let describe = index_of_tool(&lines, "Describe");
    let notes = index_of_tool(&lines, "Notes");
    assert!(read < describe, "{lines:#?}");
    assert!(read < notes, "{lines:#?}");
}

#[then(
    expr = "tools with prefix or word-boundary matches for {string} are ranked before tools with only scattered character matches"
)]
fn prefix_and_boundary_matches_precede_scattered(world: &mut TuiWorld, needle: String) {
    let lines = filtered_tool_lines(world);
    assert!(needle == "web", "unexpected fixture query: {needle}");
    let web_fetch = index_of_tool(&lines, "Web Fetch");
    let fetch_web = index_of_tool(&lines, "Fetch Web");
    let scattered = index_of_tool(&lines, "Workflow Event Bridge");
    assert!(web_fetch < scattered, "{lines:#?}");
    assert!(fetch_web < scattered, "{lines:#?}");
}

#[then(
    expr = "the Web Fetch tool is ranked before tools that mention {string} and {string} only in unrelated description text"
)]
fn web_fetch_precedes_separate_description_mentions(
    world: &mut TuiWorld,
    first: String,
    second: String,
) {
    let lines = filtered_tool_lines(world);
    assert!(
        first == "web" && second == "fetch",
        "unexpected fixture query: {first} {second}"
    );
    let web_fetch = index_of_tool(&lines, "Web Fetch");
    let remote = index_of_tool(&lines, "Remote");
    assert!(web_fetch < remote, "{lines:#?}");
}

#[then(expr = "the bash tool is ranked before tools without the {string} alias")]
fn bash_alias_precedes_non_alias(world: &mut TuiWorld, alias: String) {
    let lines = filtered_tool_lines(world);
    assert!(alias == "shell", "unexpected fixture alias: {alias}");
    let bash = index_of_tool(&lines, "Bash");
    let remote_shell = index_of_tool(&lines, "Remote Shell Notes");
    assert!(bash < remote_shell, "{lines:#?}");
}
