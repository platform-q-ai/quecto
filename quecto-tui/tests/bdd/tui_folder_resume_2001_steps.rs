//! Native TUI BDD acceptance slice for folder-aware resume (#2001).
//!
//! Every step drives existing `TuiHarness` behavior. The RED assertion observes
//! the real resume overlay returned by `list_sessions`; the compatibility
//! scenarios exercise the existing exact-key command and Ctrl+G key paths.

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::protocol::client::Event;
use quecto_tui::shell::app::tui_harness::TuiHarness;
use quecto_tui::shell::keys::Key;

fn init_harness(world: &mut TuiWorld) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let harness = runtime.block_on(TuiHarness::new());
    world.tui_parity_rt = Some(runtime);
    world.tui_parity = Some(TuiParityHarness(harness));
    world.tui_last_commands.clear();
}

fn drive<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let _guard = handle.enter();
    f(&mut world.tui_parity.as_mut().expect("TUI harness").0)
}

fn drain_commands(world: &mut TuiWorld) -> Vec<String> {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let harness = &mut world.tui_parity.as_mut().expect("TUI harness").0;
    handle.block_on(harness.drain_commands())
}

fn is_command_type(line: &str, expected: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|value| {
            value
                .get("type")
                .and_then(|field| field.as_str())
                .map(str::to_owned)
        })
        .is_some_and(|actual| actual == expected)
}

#[when("bare resume returns two persisted sessions")]
fn when_bare_resume_returns_sessions(world: &mut TuiWorld) {
    drive(world, |harness| {
        harness.submit("/resume");
    });
    let commands = drain_commands(world);
    let list_request = commands
        .iter()
        .find(|line| is_command_type(line, "list_sessions"))
        .unwrap_or_else(|| {
            panic!("bare /resume must request list_sessions; commands={commands:?}")
        });
    let request: serde_json::Value =
        serde_json::from_str(list_request).expect("list_sessions command JSON");
    let id = request
        .get("id")
        .and_then(|value| value.as_str())
        .map(str::to_owned);

    world.stdout = drive(world, |harness| {
        harness.event(Event::Response {
            id,
            command: "list_sessions".into(),
            success: true,
            data: Some(serde_json::json!({
                "sessions": [
                    {
                        "key": "cli:local-session",
                        "title": "Local investigation",
                        "messageCount": 3,
                        "updatedUnixSecs": 2
                    },
                    {
                        "key": "cli:other-folder",
                        "title": "Other folder investigation",
                        "messageCount": 5,
                        "updatedUnixSecs": 1
                    }
                ]
            })),
            error: None,
        });
        harness.full_frame()
    });
}

#[then(expr = "the resume picker shows the scope control {string}")]
fn then_resume_picker_shows_scope_control(world: &mut TuiWorld, expected: String) {
    assert!(
        world.stdout.contains("Resume session"),
        "precondition: the real resume overlay must be open; frame:\n{}",
        world.stdout
    );
    assert!(
        world.stdout.contains(&expected),
        "#2001 requires a visible Local/Global resume scope control; expected {expected:?}; frame:\n{}",
        world.stdout
    );
}

#[when(expr = "I resume the exact opaque session key {string}")]
fn when_resume_exact_key(world: &mut TuiWorld, key: String) {
    drive(world, |harness| {
        harness.submit(&format!("/resume {key}"));
    });
    world.tui_last_commands = drain_commands(world);
}

#[then(expr = "the exact opaque session key {string} is sent for resume")]
fn then_exact_key_sent(world: &mut TuiWorld, expected_key: String) {
    let resume = world
        .tui_last_commands
        .iter()
        .find(|line| is_command_type(line, "resume_session"))
        .unwrap_or_else(|| {
            panic!(
                "exact /resume <key> must send resume_session; commands={:?}",
                world.tui_last_commands
            )
        });
    let command: serde_json::Value =
        serde_json::from_str(resume).expect("resume_session command JSON");
    assert_eq!(
        command.get("session").and_then(|value| value.as_str()),
        Some(expected_key.as_str()),
        "exact opaque key must remain a global direct lookup"
    );
}

#[given("a long TUI conversation scrolled away from its latest message")]
fn given_scrolled_conversation(world: &mut TuiWorld) {
    init_harness(world);
    world.stdout = drive(world, |harness| {
        for index in 0..40 {
            harness.add_user_message(&format!("history line {index}"));
        }
        harness.press(Key::PageUp);
        harness.full_frame()
    });
    assert!(
        world
            .stdout
            .contains("Ctrl + G - Jump Back to Latest Message"),
        "precondition: PageUp must leave the real conversation scrolled; frame:\n{}",
        world.stdout
    );
}

#[when("I press Ctrl+G in the conversation")]
fn when_press_ctrl_g(world: &mut TuiWorld) {
    world.stdout = drive(world, |harness| {
        harness.press(Key::Ctrl('g'));
        harness.full_frame()
    });
}

#[then("the TUI returns to the latest conversation message")]
fn then_returns_to_latest(world: &mut TuiWorld) {
    assert!(
        world.stdout.contains("history line 39"),
        "Ctrl+G must reveal the newest conversation message; frame:\n{}",
        world.stdout
    );
    assert!(
        !world
            .stdout
            .contains("Ctrl + G - Jump Back to Latest Message"),
        "Ctrl+G must clear the scrolled-away affordance; frame:\n{}",
        world.stdout
    );
}
