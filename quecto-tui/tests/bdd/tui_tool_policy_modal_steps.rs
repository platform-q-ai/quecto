use cucumber::{given, then, when};
use quecto_tui::protocol::client::{ToolCatalogueEntry, ToolScope};
use quecto_tui::shell::keys::Key;

use crate::{TuiParityHarness, TuiWorld};

fn ensure_harness(world: &mut TuiWorld) {
    if world.tui_parity_rt.is_none() {
        world.tui_parity_rt = Some(tokio::runtime::Runtime::new().expect("tokio runtime"));
    }
    if world.tui_parity.is_none() {
        let rt = world.tui_parity_rt.as_ref().expect("runtime");
        let h = rt.block_on(async { quecto_tui::shell::app::tui_harness::TuiHarness::new().await });
        world.tui_parity = Some(TuiParityHarness(h));
    }
}

fn drain_commands(world: &mut TuiWorld) -> Vec<String> {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let h = &mut world.tui_parity.as_mut().expect("harness").0;
    handle.block_on(h.drain_commands())
}

fn complete_tool_policy_catalogue_refresh(world: &mut TuiWorld) {
    world
        .tui_parity
        .as_mut()
        .expect("harness")
        .0
        .merge_tool_catalogue(vec![
            ToolCatalogueEntry {
                stable_id: "parent-tool".into(),
                name: "parent".into(),
                profile_scope: Some(ToolScope::Child),
                ..Default::default()
            },
            ToolCatalogueEntry {
                stable_id: "child-tool".into(),
                name: "child".into(),
                profile_scope: Some(ToolScope::Child),
                ..Default::default()
            },
        ]);
    let _ = drain_commands(world);
}

fn frame(world: &mut TuiWorld) -> String {
    world.tui_parity.as_mut().expect("harness").0.full_frame()
}

#[given("the TUI has a tool catalogue with parent and child scoped tools")]
fn tui_has_tool_catalogue_with_parent_and_child_scoped_tools(world: &mut TuiWorld) {
    ensure_harness(world);
    world
        .tui_parity
        .as_mut()
        .expect("harness")
        .0
        .merge_tool_catalogue(vec![
            ToolCatalogueEntry {
                stable_id: "parent-tool".into(),
                name: "parent".into(),
                profile_scope: Some(ToolScope::Parent),
                ..Default::default()
            },
            ToolCatalogueEntry {
                stable_id: "child-tool".into(),
                name: "child".into(),
                profile_scope: Some(ToolScope::Child),
                ..Default::default()
            },
        ]);
}
#[when("the user opens the tool policy selector and applies changes")]
fn user_opens_tool_policy_selector_and_applies_changes(world: &mut TuiWorld) {
    world
        .tui_parity
        .as_mut()
        .expect("harness")
        .0
        .press(Key::Ctrl('t'));
    complete_tool_policy_catalogue_refresh(world);
    world
        .tui_parity
        .as_mut()
        .expect("harness")
        .0
        .press(Key::Char(' '));
    world
        .tui_parity
        .as_mut()
        .expect("harness")
        .0
        .press(Key::Enter);
    world.tui_last_commands = drain_commands(world);
}

#[then("the TUI sends live tool policy mutations")]
fn tui_sends_live_tool_policy_mutations(world: &mut TuiWorld) {
    let sent = world.tui_last_commands.join("\n");
    assert!(sent.contains("\"type\":\"set_tool_policy\""), "{sent}");
}

#[then("the updated catalogue availability is reflected in the TUI without restart")]
fn updated_catalogue_availability_reflected_without_restart(world: &mut TuiWorld) {
    world
        .tui_parity
        .as_mut()
        .expect("harness")
        .0
        .merge_tool_policy_results(vec![quecto_tui::protocol::client::ToolPolicyResult {
            after: Some(ToolCatalogueEntry {
                stable_id: "parent-tool".into(),
                name: "parent".into(),
                profile_scope: Some(ToolScope::Child),
                ..Default::default()
            }),
            ..Default::default()
        }]);
    world
        .tui_parity
        .as_mut()
        .expect("harness")
        .0
        .press(Key::Ctrl('t'));
    complete_tool_policy_catalogue_refresh(world);
    assert!(quecto_tui::components::ansi::strip_ansi(&frame(world)).contains("[-C] parent"));
}

#[given("the TUI help is shown")]
fn tui_help_is_shown(world: &mut TuiWorld) {
    ensure_harness(world);
    world
        .tui_parity
        .as_mut()
        .expect("harness")
        .0
        .show_help_frame();
}

#[then("the help mentions Ctrl+T for tool policy")]
fn help_mentions_ctrl_t_for_tool_policy(world: &mut TuiWorld) {
    assert!(
        quecto_tui::components::ansi::strip_ansi(&frame(world))
            .contains("Ctrl+T         Open tool policy selector")
    );
}
