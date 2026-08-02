use cucumber::{given, then, when};
use quecto_tui::protocol::client::{ToolCatalogueEntry, ToolScope};
use quecto_tui::shell::keys::Key;

use crate::{TuiParityHarness, TuiWorld};

fn ensure_sized_harness(world: &mut TuiWorld, width: usize) {
    if world.tui_parity_rt.is_none() {
        world.tui_parity_rt = Some(tokio::runtime::Runtime::new().expect("tokio runtime"));
    }
    let rt = world.tui_parity_rt.as_ref().expect("runtime");
    world.tui_parity = Some(TuiParityHarness(rt.block_on(async {
        quecto_tui::shell::app::tui_harness::TuiHarness::sized(width, 32).await
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

fn open_tool_management(world: &mut TuiWorld) {
    h(world).press(Key::Ctrl('t'));
    let _ = drain_commands(world);
    h(world).merge_tool_catalogue(vec![
        ToolCatalogueEntry {
            stable_id: "tool-alpha".into(),
            name: "alpha".into(),
            profile_scope: Some(ToolScope::None),
            ..Default::default()
        },
        ToolCatalogueEntry {
            stable_id: "tool-beta".into(),
            name: "beta".into(),
            profile_scope: Some(ToolScope::Parent),
            ..Default::default()
        },
    ]);
}

fn plain_frame(world: &mut TuiWorld) -> String {
    quecto_tui::components::ansi::strip_ansi(&h(world).full_frame())
}

#[given("the TUI has a current tool catalogue")]
fn tui_has_current_tool_catalogue(world: &mut TuiWorld) {
    ensure_sized_harness(world, 100);
}

#[when("the user opens tool management")]
fn user_opens_tool_management(world: &mut TuiWorld) {
    open_tool_management(world);
}

#[then("the modal shows each available tool with its current enabled state")]
fn modal_shows_tools_with_current_state(world: &mut TuiWorld) {
    let frame = plain_frame(world);
    assert!(frame.contains("[--] alpha"), "{frame}");
    assert!(frame.contains("[P-] beta"), "{frame}");
}

#[then("the modal help distinguishes parent-and-child allowance from disabling visible tools")]
fn modal_help_shows_bulk_shortcuts(world: &mut TuiWorld) {
    let frame = plain_frame(world);
    assert!(frame.contains("Ctrl+Shift+A allow all"), "{frame}");
    assert!(frame.contains("Ctrl+Shift+D disable matches"), "{frame}");
    assert!(!frame.contains("disable visible"), "{frame}");
}

#[when("the user changes tool access for a tool")]
fn user_changes_tool_access_for_a_tool(world: &mut TuiWorld) {
    open_tool_management(world);
    h(world).press(Key::Char(' ')).press(Key::Enter);
    world.tui_last_commands = drain_commands(world);
}

#[then("the TUI sends the updated master tool access configuration to the kernel")]
fn tui_sends_updated_master_tool_access(world: &mut TuiWorld) {
    let sent = world.tui_last_commands.join("\n");
    assert!(sent.contains("\"type\":\"set_tool_policy\""), "{sent}");
    assert!(sent.contains("\"scope\":\"parent\""), "{sent}");
}

#[then("child-agent tool access is unchanged")]
fn child_agent_tool_access_unchanged(world: &mut TuiWorld) {
    let sent = world.tui_last_commands.join("\n");
    assert!(!sent.contains("\"scope\":\"child\""), "{sent}");
    assert!(!sent.contains("\"scope\":\"both\""), "{sent}");
}

#[given("the TUI has many available tools")]
fn tui_has_many_available_tools(world: &mut TuiWorld) {
    ensure_sized_harness(world, 120);
    h(world).press(Key::Ctrl('t'));
    let _ = drain_commands(world);
    h(world).merge_tool_catalogue(
        (0..25)
            .map(|idx| ToolCatalogueEntry {
                stable_id: format!("tool-{idx:02}"),
                name: format!("tool {idx:02}"),
                profile_scope: Some(ToolScope::None),
                ..Default::default()
            })
            .collect(),
    );
}

#[when("the user opens tool management on a wide terminal")]
fn user_opens_tool_management_on_wide_terminal(_world: &mut TuiWorld) {}

#[then("the modal shows tools in two columns")]
fn modal_shows_tools_in_two_columns(world: &mut TuiWorld) {
    let frame = plain_frame(world);
    assert!(
        frame
            .lines()
            .any(|line| line.contains("tool 00") && line.contains("tool 12")),
        "{frame}"
    );
}

#[then(
    "filtering, selection state, navigation, and bulk shortcuts still apply to the visible tools"
)]
fn two_column_behaviour_remains_available(world: &mut TuiWorld) {
    h(world).press(Key::Down);
    let navigated = plain_frame(world);
    assert!(
        navigated.lines().filter(|line| line.contains('→')).count() == 1,
        "{navigated}"
    );
    assert!(
        navigated
            .lines()
            .any(|line| line.contains('→') && line.contains("tool 01")),
        "{navigated}"
    );

    for c in "tool 24".chars() {
        h(world).press(Key::Char(c));
    }
    let filtered = plain_frame(world);
    assert!(filtered.contains("tool 24"), "{filtered}");
    assert!(!filtered.contains("tool 00"), "{filtered}");

    h(world)
        .press(Key::CtrlShift('a'))
        .press(Key::CtrlShift('d'))
        .press(Key::Enter);
    let sent = drain_commands(world).join("\n");
    assert!(sent.contains("\"type\":\"set_tool_policy\""), "{sent}");
    assert!(sent.contains("\"name\":\"tool 24\""), "{sent}");
    assert!(sent.contains("\"scope\":\"none\""), "{sent}");
}
