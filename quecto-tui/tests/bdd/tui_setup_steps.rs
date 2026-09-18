//! Steps for `tui_setup.feature` (#2024 S6): `/setup` composes the
//! agent-executable setup walkthrough and submits it as an ordinary user turn.
//!
//! The scenarios drive the real `handle_submit` path through the headless
//! harness and assert the observable outcome: the transcript's user entry, the
//! serialized `prompt` command, the usage toast, the help listing and the
//! autocomplete set. The prompt text itself comes from the `setup` feature
//! module, and the safety scenario pins the affirmative rules by their wording.

use crate::TuiWorld;
use cucumber::then;
use quecto_tui::setup::{SETUP_USAGE, SetupCommand, setup_walkthrough_prompt};
use quecto_tui::shell::app::tui_harness::TuiHarness;

fn drive<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let _guard = handle.enter();
    let h = &mut world.tui_parity.as_mut().expect("harness").0;
    f(h)
}

/// The prompt the feature file names by its `/setup` argument text
/// (`"all areas"` for the bare command).
fn expected_prompt(variant: &str) -> String {
    let args = if variant == "all areas" { "" } else { variant };
    match SetupCommand::parse(args) {
        SetupCommand::Walkthrough(area) => setup_walkthrough_prompt(&area),
        SetupCommand::Usage => panic!("{variant:?} is not a walkthrough variant"),
    }
}

fn prompt_commands(commands: &[String]) -> Vec<serde_json::Value> {
    commands
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("prompt"))
        .collect()
}

#[then(
    expr = "the master transcript shows the setup walkthrough prompt for {string} as the user's turn"
)]
fn then_transcript_shows_prompt(world: &mut TuiWorld, variant: String) {
    let expected = expected_prompt(&variant);
    let entries = drive(world, |h| h.active_user_entries());
    assert_eq!(
        entries,
        vec![expected.clone()],
        "the walkthrough must be the one visible user turn"
    );
    // And it is painted, not just stored: the rendered (wrapped) chat carries
    // the prompt's opening words.
    let opening: String = expected.chars().take(48).collect();
    let rendered = drive(world, |h| h.active_chat_text(200));
    assert!(
        rendered.contains(&opening),
        "rendered chat should show the prompt's opening {opening:?}, got:\n{rendered}"
    );
}

#[then(expr = "a prompt command is sent carrying the setup walkthrough prompt for {string}")]
fn then_prompt_command_sent(world: &mut TuiWorld, variant: String) {
    let expected = expected_prompt(&variant);
    let prompts = prompt_commands(&world.tui_last_commands);
    assert_eq!(
        prompts.len(),
        1,
        "exactly one prompt command, got {:?}",
        world.tui_last_commands
    );
    assert_eq!(
        prompts[0].get("message").and_then(|m| m.as_str()),
        Some(expected.as_str()),
        "the prompt command must carry the walkthrough verbatim"
    );
}

#[then(
    expr = "the master follow-up command is sent with the setup walkthrough prompt for {string}"
)]
fn then_follow_up_carries_prompt(world: &mut TuiWorld, variant: String) {
    let expected = expected_prompt(&variant);
    let follow_ups: Vec<serde_json::Value> = world
        .tui_last_commands
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("follow_up"))
        .collect();
    assert_eq!(
        follow_ups.len(),
        1,
        "a streaming master must queue exactly one follow_up, got {:?}",
        world.tui_last_commands
    );
    assert_eq!(
        follow_ups[0].get("message").and_then(|m| m.as_str()),
        Some(expected.as_str()),
        "the follow_up must carry the walkthrough verbatim"
    );
}

#[then(expr = "sub-agent {string} received no setup command")]
fn then_subagent_received_nothing(world: &mut TuiWorld, id: String) {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let rx = world
        .tui_subagent_commands
        .as_mut()
        .expect("sub-agent command receiver");
    let cmds: Vec<String> = handle.block_on(async {
        let mut cmds = Vec::new();
        while let Ok(cmd) = rx.try_recv() {
            cmds.push(cmd);
        }
        cmds
    });
    // The attach handshake (`get_state`/`sync`) is fine; no user text may
    // reach the child.
    let user_text: Vec<&String> = cmds
        .iter()
        .filter(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_owned))
                .is_some_and(|t| t == "prompt" || t == "follow_up")
        })
        .collect();
    assert!(
        user_text.is_empty(),
        "sub-agent {id} must receive no prompt/follow_up from /setup, got {cmds:?}"
    );
}

#[then("the selected sub-agent transcript has no user turn")]
fn then_subagent_no_user_turn(world: &mut TuiWorld) {
    let entries = drive(world, |h| h.active_user_entries());
    assert!(
        entries.is_empty(),
        "the focused sub-agent must not get the walkthrough, got {entries:?}"
    );
}

#[then(expr = "the setup walkthrough prompt for {string} names the docs page {string}")]
fn then_prompt_names_page(_world: &mut TuiWorld, variant: String, page: String) {
    let prompt = expected_prompt(&variant);
    let needle = format!("docs {{\"name\":\"{page}\"}}");
    assert!(
        prompt.contains(&needle),
        "prompt for {variant:?} should name {needle}, got:\n{prompt}"
    );
    if let Some(model) = variant.strip_prefix("model ") {
        assert!(
            prompt.contains(model),
            "the model variant must name the requested model id {model:?}"
        );
    }
}

#[then("a setup usage toast lists the variants")]
fn then_usage_toast(world: &mut TuiWorld) {
    let toasts = drive(world, |h| h.notification_messages());
    assert!(
        toasts.iter().any(|t| t == SETUP_USAGE),
        "usage toast expected, got {toasts:?}"
    );
    for variant in ["model <model-id>", "admission", "podman", "auth"] {
        assert!(
            SETUP_USAGE.contains(variant),
            "usage should list {variant:?}: {SETUP_USAGE}"
        );
    }
}

#[then("no prompt command is sent")]
fn then_no_prompt_command(world: &mut TuiWorld) {
    let mut commands = world.tui_last_commands.clone();
    commands.extend(drive(world, |h| h.try_drain_commands()));
    assert!(
        prompt_commands(&commands).is_empty(),
        "no prompt should be sent, got {commands:?}"
    );
}

#[then("the master transcript has no user turn")]
fn then_no_user_turn(world: &mut TuiWorld) {
    let entries = drive(world, |h| h.active_user_entries());
    assert!(entries.is_empty(), "no user turn expected, got {entries:?}");
}

#[then(expr = "the help listing shows {string}")]
fn then_help_lists(world: &mut TuiWorld, command: String) {
    let help = drive(world, |h| h.show_help_frame());
    assert!(
        help.lines().any(|l| l.trim_start().starts_with(&command)),
        "help should list {command}, got:\n{help}"
    );
}

#[then(expr = "the slash-command autocomplete offers {string}")]
fn then_autocomplete_offers(_world: &mut TuiWorld, name: String) {
    let names = TuiHarness::slash_command_names();
    assert!(
        names.contains(&name),
        "autocomplete should offer {name}: {names:?}"
    );
}
