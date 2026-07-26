//! Steps for `tui_border_replication.feature` — assert the editor renders
//! exactly one top and one bottom border across repeated renders / paste, and
//! that the diff renderer repaints with absolute cursor addressing on the
//! alternate screen (no scrollback-disturbing full clear or relative stepping).

use std::sync::{Arc, Mutex};

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::components::component::Component;
use quecto_tui::components::editor::Editor;
use quecto_tui::interface::ansi::sanitize_control;
use quecto_tui::interface::app::tui_harness::TuiHarness;
use quecto_tui::protocol::client::Event;
use quecto_tui::shell::keys::Key;
use quecto_tui::shell::render::DiffRenderer;

fn drain_commands(world: &mut TuiWorld) -> Vec<String> {
    if let Some(rt) = &world.tui_parity_rt {
        if let Some(h) = world.tui_parity.as_mut() {
            return rt.block_on(h.0.drain_commands());
        }
    }
    Vec::new()
}

fn command_of_type<'a>(commands: &'a [String], expected_type: &str) -> Option<&'a str> {
    commands.iter().map(String::as_str).find(|line| {
        serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string))
            .as_deref()
            == Some(expected_type)
    })
}

/// Count (top, bottom) editor borders in a rendered block. The editor's top
/// border carries the ` > ` (or ` ! `) prompt indicator between horizontal
/// rules; its bottom border is a full run of `─`.
fn count_borders(lines: &[String]) -> (usize, usize) {
    let mut top = 0;
    let mut bottom = 0;
    for line in lines {
        let s = sanitize_control(line);
        let t = s.trim();
        if t.is_empty() || !t.contains('─') {
            continue;
        }
        if t.contains('>') || t.contains('!') {
            top += 1;
        } else if t.chars().all(|c| c == '─') {
            bottom += 1;
        }
    }
    (top, bottom)
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

// ── Scenario 1: borders stable during a streaming response (App-level) ────────

#[given("the agent is streaming tokens")]
fn agent_streaming_tokens(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.event(Event::AgentStart);
        for _ in 0..20 {
            h.event(Event::Token { token: "x".into() });
        }
        h.set_streaming(true);
    });
}

#[given("the spinner is active above the editor")]
fn spinner_active(world: &mut TuiWorld) {
    with_harness(world, |h| h.show_activity_spinner("Working"));
}

#[when("the screen re-renders on each token")]
fn rerender_each_token(world: &mut TuiWorld) {
    // Drive several more token renders through the real render path.
    with_harness(world, |h| {
        for _ in 0..10 {
            h.event(Event::Token { token: "y".into() });
        }
    });
}

#[then("the editor should show exactly one top border and one bottom border")]
fn editor_one_top_one_bottom(world: &mut TuiWorld) {
    let stack = with_harness(world, |h| h.bottom_stack());
    let lines: Vec<String> = stack.lines().map(str::to_string).collect();
    let (top, bottom) = count_borders(&lines);
    assert_eq!(
        top, 1,
        "editor should render exactly one top border, found {top} in:\n{stack}"
    );
    assert_eq!(
        bottom, 1,
        "editor should render exactly one bottom border, found {bottom} in:\n{stack}"
    );
}

// ── Scenario 2: multi-line paste keeps a single clean frame (component-level) ─

#[given(regex = r#"^an editor component with text "([^"]*)"$"#)]
fn editor_component_with_text(world: &mut TuiWorld, text: String) {
    let mut editor = Editor::new();
    if !text.is_empty() {
        editor.set_text(&text);
    }
    world.tui_editor = Some(crate::DebugEditor(editor));
    world.tui_editor_renders = Vec::new();
}

#[when(regex = r#"^the user pastes "(.*)"$"#)]
fn user_pastes(world: &mut TuiWorld, text: String) {
    // The feature encodes newlines as literal \r\n; feed the real bytes through
    // the production bracketed-paste handler (Key::Paste).
    let payload = text.replace("\\r", "\r").replace("\\n", "\n");
    let editor = world.tui_editor.as_mut().expect("editor component");
    editor.handle_input(&Key::Paste(payload));
}

#[when(regex = r#"^the editor renders at width (\d+) three times$"#)]
fn editor_renders_thrice(world: &mut TuiWorld, width: usize) {
    let editor = world.tui_editor.as_mut().expect("editor component");
    for _ in 0..3 {
        // Force a fresh layout each time so a border-duplication regression in
        // render would actually surface rather than returning a stale cache.
        editor.invalidate();
        world.tui_editor_renders.push(editor.render(width));
    }
}

#[then("each render should show exactly one top border and one bottom border")]
fn each_render_one_top_one_bottom(world: &mut TuiWorld) {
    assert_eq!(world.tui_editor_renders.len(), 3, "expected three renders");
    for (i, render) in world.tui_editor_renders.iter().enumerate() {
        let (top, bottom) = count_borders(render);
        assert_eq!(
            top, 1,
            "render {i} should have exactly one top border: {render:?}"
        );
        assert_eq!(
            bottom, 1,
            "render {i} should have exactly one bottom border: {render:?}"
        );
    }
}

#[then(regex = r#"^the rendered output should contain "([^"]*)"$"#)]
fn rendered_output_contains(world: &mut TuiWorld, needle: String) {
    let joined: String = world
        .tui_editor_renders
        .iter()
        .flat_map(|r| r.iter())
        .map(|l| sanitize_control(l))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains(&needle),
        "pasted content {needle:?} should appear in the rendered editor: {joined:?}"
    );
}

#[when(expr = "I type the prompt keys {string}")]
fn type_prompt_keys(world: &mut TuiWorld, text: String) {
    with_harness(world, |h| {
        for ch in text.chars() {
            h.press(Key::Char(ch));
        }
    });
}

#[when("I press Shift Enter in the editor")]
fn press_shift_enter(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.press(Key::ShiftEnter);
    });
}

#[when("I press Enter in the editor")]
fn press_enter_in_editor(world: &mut TuiWorld) {
    with_harness(world, |h| {
        h.press(Key::Enter);
    });
    world.tui_last_commands = drain_commands(world);
}

#[then(expr = "the master prompt command message is {string}")]
fn master_prompt_command_message_is(world: &mut TuiWorld, expected: String) {
    let expected = expected.replace("\\n", "\n");
    let prompt = command_of_type(&world.tui_last_commands, "prompt")
        .unwrap_or_else(|| panic!("expected prompt command, got {:?}", world.tui_last_commands));
    let value: serde_json::Value = serde_json::from_str(prompt).expect("prompt command json");
    assert_eq!(
        value.get("message").and_then(|v| v.as_str()),
        Some(expected.as_str()),
        "submitted prompt should preserve editor newlines: {prompt}"
    );
}

// ── Scenario 3: alternate screen buffer / cursor home (DiffRenderer) ──────────

struct SharedWriter(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[given("the TUI uses the alternate screen buffer")]
fn tui_uses_alt_screen(world: &mut TuiWorld) {
    world.tui_render_full = None;
    world.tui_render_diff = None;
}

#[when("content is rendered via cursor home")]
fn content_rendered_via_home(world: &mut TuiWorld) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let mut renderer = DiffRenderer::new(SharedWriter(buf.clone()));

    renderer
        .render(&["line one".to_string(), "line two".to_string()], 40)
        .expect("full render");
    let full = String::from_utf8(buf.lock().unwrap().clone()).expect("utf8");
    buf.lock().unwrap().clear();

    renderer
        .render(&["line one".to_string(), "CHANGED two".to_string()], 40)
        .expect("diff render");
    let diff = String::from_utf8(buf.lock().unwrap().clone()).expect("utf8");

    world.tui_render_full = Some(full);
    world.tui_render_diff = Some(diff);
}

#[then("scrollback does not cause position errors")]
fn scrollback_no_position_errors(world: &mut TuiWorld) {
    let full = world
        .tui_render_full
        .as_ref()
        .expect("full render captured");
    let diff = world
        .tui_render_diff
        .as_ref()
        .expect("diff render captured");
    assert!(
        full.contains("\u{1b}[H"),
        "the full render must home the cursor to a known origin: {full:?}"
    );
    assert!(
        !full.contains("\u{1b}[2J"),
        "the first render must not clear the screen (no scrollback disturbance): {full:?}"
    );
    assert!(
        diff.contains(";1H"),
        "the diff render must repaint with absolute cursor addressing, not \
         viewport-scrolling relative stepping: {diff:?}"
    );
}
