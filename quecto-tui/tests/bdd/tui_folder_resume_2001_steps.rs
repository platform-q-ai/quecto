//! Operational native-TUI acceptance coverage for folder-aware resume (#2001).
//!
//! The tests deliberately use only the existing `TuiHarness` surface: submit a
//! command, inject real protocol responses, press decoded keys/mouse events,
//! render the real frame, and drain real commands. The technically approved
//! minimal contract supplies only `scope`/`query` for listing and exact request
//! correlation; tests avoid exhaustive rows, generations, and guessed focus
//! ordering.

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::protocol::client::Event;
use quecto_tui::shell::app::tui_harness::TuiHarness;
use quecto_tui::shell::keys::Key;
use serde_json::{Value, json};

const STABLE_KEY: &str = "cli:stable-session";

fn init_harness(world: &mut TuiWorld) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let harness = runtime.block_on(TuiHarness::new());
    world.tui_parity_rt = Some(runtime);
    world.tui_parity = Some(TuiParityHarness(harness));
    world.tui_last_commands.clear();
    world.stdout.clear();
    world.stderr.clear();
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

fn command_json(line: &str) -> Option<Value> {
    serde_json::from_str(line).ok()
}

fn command_type(value: &Value) -> Option<&str> {
    value.get("type").and_then(Value::as_str)
}

fn list_requests(commands: &[String]) -> Vec<Value> {
    commands
        .iter()
        .filter_map(|line| command_json(line))
        .filter(|value| command_type(value) == Some("list_sessions"))
        .collect()
}

fn list_request(commands: &[String]) -> Option<Value> {
    list_requests(commands).into_iter().next()
}

fn list_scope(request: &Value) -> Option<&str> {
    request.get("scope").and_then(Value::as_str)
}

fn is_local_default(request: &Value) -> bool {
    matches!(list_scope(request), None | Some("local"))
}

fn is_global(request: &Value) -> bool {
    list_scope(request) == Some("global")
}

fn response_id(request: &Value) -> Option<String> {
    request.get("id").and_then(Value::as_str).map(str::to_owned)
}

fn session(key: &str, title: &str, updated: u64) -> Value {
    json!({
        "key": key,
        "title": title,
        "messageCount": 3,
        "updatedUnixSecs": updated
    })
}

fn session_at(key: &str, title: &str, updated: u64, path: &str) -> Value {
    let mut row = session(key, title, updated);
    row["path"] = json!(path);
    row
}

fn inject_sessions(world: &mut TuiWorld, id: Option<String>, sessions: Vec<Value>) -> String {
    drive(world, |harness| {
        harness.event(Event::Response {
            id,
            command: "list_sessions".into(),
            success: true,
            data: Some(json!({ "sessions": sessions })),
            error: None,
        });
        harness.full_frame()
    })
}

/// Open the picker through its public command/response path and return the
/// actual fieldless Local request.  The caller supplies rows so each scenario
/// controls only observable data, never application internals.
fn open_picker(world: &mut TuiWorld, sessions: Vec<Value>) -> Value {
    drive(world, |harness| {
        harness.submit("/resume");
    });
    let commands = drain_commands(world);
    let request = list_request(&commands).unwrap_or_else(|| {
        panic!("bare /resume must request list_sessions; commands={commands:?}")
    });
    world.stdout = inject_sessions(world, response_id(&request), sessions);
    request
}

fn resume_key(commands: &[String]) -> Option<String> {
    commands
        .iter()
        .filter_map(|line| command_json(line))
        .find(|value| command_type(value) == Some("resume_session"))
        .and_then(|value| {
            value
                .get("session")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
}

fn type_text(world: &mut TuiWorld, text: &str) {
    for ch in text.chars() {
        drive(world, |harness| {
            harness.press(Key::Char(ch));
        });
    }
}

/// Find the query by reachability, not by assuming which of the three controls
/// owns initial focus or what order the implementation uses for them.
fn query_by_reachability(world: &mut TuiWorld, query: &str) -> Option<String> {
    for tabs in 0..3 {
        init_harness(world);
        let initial = open_picker(
            world,
            vec![
                session("cli:KEY-NEEDLE", "First unrelated title", 2),
                session("cli:other", "Second unrelated title", 1),
            ],
        );
        for _ in 0..tabs {
            drive(world, |harness| {
                harness.press(Key::Tab);
            });
        }
        type_text(world, query);
        let commands = drain_commands(world);
        let Some(request) = list_requests(&commands).into_iter().find(|request| {
            request.get("query").and_then(Value::as_str) == Some(query)
                && response_id(request) != response_id(&initial)
        }) else {
            continue;
        };
        let frame = inject_sessions(
            world,
            response_id(&request),
            vec![session("cli:KEY-NEEDLE", "First unrelated title", 2)],
        );
        return Some(frame);
    }
    None
}

/// Find Global by reachability, not by assuming an initial focus or traversal
/// order. A genuine Global activation is identified by the approved `scope`
/// value rather than an implementation-specific payload shape.
fn activate_global_by_keyboard(world: &mut TuiWorld, activation: Key) -> Option<Value> {
    for tabs in 0..3 {
        init_harness(world);
        open_picker(world, vec![session("cli:local", "Local result", 1)]);
        for _ in 0..tabs {
            drive(world, |harness| {
                harness.press(Key::Tab);
            });
        }
        drive(world, |harness| {
            harness.press(activation.clone());
        });
        let commands = drain_commands(world);
        if let Some(global) = list_requests(&commands).into_iter().find(is_global) {
            return Some(global);
        }
    }
    None
}

fn activate_global_and_inject(world: &mut TuiWorld, activation: Key, title: &str) -> bool {
    let Some(request) = activate_global_by_keyboard(world, activation) else {
        return false;
    };
    world.stdout = inject_sessions(
        world,
        response_id(&request),
        vec![session("cli:global-only", title, 1)],
    );
    world.stdout.contains(title)
}

#[when("bare resume returns two persisted sessions")]
fn when_bare_resume_returns_sessions(world: &mut TuiWorld) {
    let request = open_picker(
        world,
        vec![
            session("cli:local-session", "Alpha investigation", 2),
            session("cli:other-folder", "Beta investigation", 1),
        ],
    );
    world.tui_last_commands = vec![request.to_string()];
}

#[then("the bare resume request uses the approved local discovery default")]
fn then_bare_resume_is_fieldless_local(world: &mut TuiWorld) {
    let request = command_json(
        world
            .tui_last_commands
            .first()
            .expect("captured bare list request"),
    )
    .expect("list request JSON");
    assert_eq!(command_type(&request), Some("list_sessions"));
    assert!(
        is_local_default(&request),
        "bare /resume must use local discovery: omission or explicit scope=local are both approved; request={request}"
    );
}

#[then("the resume picker separately shows Local and Global scope labels")]
fn then_picker_shows_loose_scope_labels(world: &mut TuiWorld) {
    assert!(
        world.stdout.contains("Resume session"),
        "precondition: real resume overlay must be open; frame:\n{}",
        world.stdout
    );
    assert!(
        world.stdout.contains("Local") && world.stdout.contains("Global"),
        "#2001 requires independently visible Local and Global labels, without prescribing decoration or exact spacing; frame:\n{}",
        world.stdout
    );
}

#[when("I activate Global in the resume picker with Enter and Space")]
fn when_activate_global_keyboard(world: &mut TuiWorld) {
    let enter_worked = activate_global_and_inject(world, Key::Enter, "Global result from Enter");
    let space_worked =
        activate_global_and_inject(world, Key::Char(' '), "Global result from Space");
    world.tui_last_commands = vec![enter_worked.to_string(), space_worked.to_string()];
}

#[then(expr = "both activations request scope {string} and present a global result")]
fn then_global_request_and_result(world: &mut TuiWorld, expected_scope: String) {
    assert_eq!(expected_scope, "global", "approved Global scope spelling");
    assert_eq!(
        world.tui_last_commands,
        ["true", "true"],
        "reachable Global must accept Enter and Space, issue list_sessions with scope=global, and present each exactly correlated result"
    );
}

#[when("I move resume picker focus forward and backward")]
fn when_move_focus_forward_backward(world: &mut TuiWorld) {
    open_picker(world, vec![session("cli:one", "One result", 1)]);
    let mut frames = vec![drive(world, TuiHarness::full_frame)];
    for _ in 0..3 {
        drive(world, |harness| {
            harness.press(Key::Tab);
        });
        frames.push(drive(world, TuiHarness::full_frame));
    }
    for _ in 0..3 {
        drive(world, |harness| {
            harness.press(Key::BackTab);
        });
        frames.push(drive(world, TuiHarness::full_frame));
    }
    world.tui_last_commands = frames;
}

#[then("each resume picker focus move is visible and reversible")]
fn then_focus_visible_reversible(world: &mut TuiWorld) {
    let frames = &world.tui_last_commands;
    assert_eq!(
        frames.len(),
        7,
        "initial + three forward + three reverse frames"
    );
    assert!(
        frames.windows(2).take(3).all(|pair| pair[0] != pair[1]),
        "each Tab must visibly reach the next modal-local control among scope, query and results"
    );
    assert_eq!(
        frames[4], frames[2],
        "first Shift+Tab must reverse the last Tab without prescribing which control that is"
    );
    assert_eq!(
        frames[5], frames[1],
        "second Shift+Tab must reverse the preceding Tab"
    );
    assert_eq!(frames[6], frames[0], "third Shift+Tab must return to start");
}

#[when("I enter a resume query that only matches an opaque session key")]
fn when_query_matches_key(world: &mut TuiWorld) {
    let frame = query_by_reachability(world, "KEY-NEEDLE");
    world.tui_last_commands = vec![frame.is_some().to_string()];
    world.stdout = frame.unwrap_or_default();
}

#[then("only the session with the matching opaque key remains in the results")]
fn then_key_query_filters_results(world: &mut TuiWorld) {
    assert_eq!(
        world.tui_last_commands,
        ["true"],
        "typing in the reachable query control must emit a fresh list_sessions request carrying query=KEY-NEEDLE"
    );
    assert!(
        world.stdout.contains("First unrelated title"),
        "the response to the correlated metadata query must retain the opaque-key match; frame:\n{}",
        world.stdout
    );
    assert!(
        !world.stdout.contains("Second unrelated title"),
        "metadata search must remove a row matching neither title nor opaque key; frame:\n{}",
        world.stdout
    );
}

fn activate_single_result(key: Key) -> Vec<String> {
    let mut world = TuiWorld::default();
    init_harness(&mut world);
    open_picker(
        &mut world,
        vec![session("cli:activation", "Activation result", 1)],
    );
    drive(&mut world, |harness| {
        harness.press(key);
    });
    drain_commands(&mut world)
}

#[when("I activate focused resume results with Enter and Space")]
fn when_activate_results_enter_space(world: &mut TuiWorld) {
    let enter_commands = activate_single_result(Key::Enter);
    let space_commands = activate_single_result(Key::Char(' '));
    world.tui_last_commands = vec![
        resume_key(&enter_commands).unwrap_or_default(),
        resume_key(&space_commands).unwrap_or_default(),
    ];
}

#[then("both focused results are requested for resume")]
fn then_enter_space_resume(world: &mut TuiWorld) {
    assert_eq!(
        world.tui_last_commands,
        ["cli:activation", "cli:activation"],
        "Enter and Space must both activate the focused result through the real command path"
    );
}

#[when("I click the Global resume scope label")]
fn when_click_global_label(world: &mut TuiWorld) {
    open_picker(world, vec![session("cli:local", "Local result", 1)]);
    let frame = drive(world, TuiHarness::full_frame);
    let hit = frame
        .lines()
        .enumerate()
        .find_map(|(row, line)| line.find("Global").map(|byte_col| (row, line, byte_col)));
    if let Some((row, line, byte_col)) = hit {
        let col = line[..byte_col].chars().count();
        drive(world, |harness| {
            harness.press(Key::MousePress(col as u16, row as u16));
            harness.press(Key::MouseRelease(col as u16, row as u16));
        });
    }
    let commands = drain_commands(world);
    let global = list_requests(&commands).into_iter().find(is_global);
    world.tui_last_commands = global
        .as_ref()
        .map(|value| value.to_string())
        .into_iter()
        .collect();
    if let Some(request) = global {
        world.stdout = inject_sessions(
            world,
            response_id(&request),
            vec![session("cli:global-only", "Global result", 1)],
        );
    } else {
        world.stdout = drive(world, TuiHarness::full_frame);
    }
}

#[then("the TUI requests another session list and presents its global result")]
fn then_mouse_global_request_and_result(world: &mut TuiWorld) {
    assert_eq!(
        world.tui_last_commands.len(),
        1,
        "clicking the visible Global hit target must emit list_sessions with scope=global"
    );
    assert!(
        world.stdout.contains("Global result"),
        "the correlated Global response must replace/present results after mouse activation; frame:\n{}",
        world.stdout
    );
}

#[when("I cancel the open resume picker with Escape")]
fn when_cancel_picker(world: &mut TuiWorld) {
    open_picker(world, vec![session("cli:cancelled", "Cancelled result", 1)]);
    drive(world, |harness| {
        harness.press(Key::Escape);
    });
    world.tui_last_commands = drain_commands(world);
    world.stdout = drive(world, TuiHarness::full_frame);
}

#[then("no session is requested for resume")]
fn then_no_resume_requested(world: &mut TuiWorld) {
    assert!(
        resume_key(&world.tui_last_commands).is_none(),
        "Escape must not send resume_session; commands={:?}",
        world.tui_last_commands
    );
    assert!(
        !world.stdout.contains("Resume session"),
        "Escape must close the modal; frame:\n{}",
        world.stdout
    );
}

/// Open Local, highlight the stable key, then reach Global through the real
/// modal. The resulting Local and Global requests provide two application-
/// generated correlation ids without assuming focus order or inventing ids.
fn local_then_global_requests(world: &mut TuiWorld) -> Option<(Value, Value)> {
    for tabs in 0..3 {
        init_harness(world);
        let local = open_picker(
            world,
            vec![
                session("cli:first", "First result", 3),
                session(STABLE_KEY, "Stable result", 2),
            ],
        );
        drive(world, |harness| {
            harness.press(Key::Down);
        });
        for _ in 0..tabs {
            drive(world, |harness| {
                harness.press(Key::Tab);
            });
        }
        drive(world, |harness| {
            harness.press(Key::Char(' '));
        });
        let commands = drain_commands(world);
        if let Some(global) = list_requests(&commands).into_iter().find(is_global)
            && response_id(&local) != response_id(&global)
        {
            return Some((local, global));
        }
    }
    None
}

#[when("newer and older correlated Global results arrive out of order around my stable selection")]
fn when_async_refresh_after_highlight(world: &mut TuiWorld) {
    let Some((older, latest)) = local_then_global_requests(world) else {
        world.tui_last_commands = vec!["local-global-requests=false".into()];
        return;
    };
    world.tui_last_commands = vec!["local-global-requests=true".into()];

    // The latest response reorders rows but retains STABLE_KEY; selection must
    // follow the key. The older response arrives afterward and must be ignored.
    inject_sessions(
        world,
        response_id(&latest),
        vec![
            session("cli:new", "Newest-only result", 4),
            session(STABLE_KEY, "Stable result", 2),
            session("cli:first", "First result", 1),
        ],
    );
    world.stdout = inject_sessions(
        world,
        response_id(&older),
        vec![session("cli:stale", "Stale late result", 9)],
    );
}

#[then("only the newest response is shown and the same stable session remains highlighted")]
fn then_stable_selection_survives_refresh(world: &mut TuiWorld) {
    assert_eq!(
        world.tui_last_commands,
        ["local-global-requests=true"],
        "precondition: Local and the subsequent scope=global request must carry fresh ids"
    );
    assert!(
        world.stdout.contains("Newest-only result") && !world.stdout.contains("Stale late result"),
        "only the exact latest request id may update picker results; frame:\n{}",
        world.stdout
    );
    let selected_line = world
        .stdout
        .lines()
        .find(|line| line.contains('→'))
        .unwrap_or_default();
    assert!(
        selected_line.contains("Stable result"),
        "the latest response must preserve selection by opaque key when rows reorder; selected line={selected_line:?}; frame:\n{}",
        world.stdout
    );
}

#[when("I select a session whose path is the current execution directory")]
fn when_select_same_execution_directory(world: &mut TuiWorld) {
    let cwd = std::env::current_dir().expect("current execution directory");
    let cwd = cwd.to_string_lossy();
    open_picker(
        world,
        vec![session_at(
            "cli:same-execution-dir",
            "Same execution directory",
            1,
            &cwd,
        )],
    );
    drive(world, |harness| {
        harness.press(Key::Enter);
    });
    let commands = drain_commands(world);
    let request = commands
        .iter()
        .filter_map(|line| command_json(line))
        .find(|value| command_type(value) == Some("resume_session"));
    world.tui_last_commands = request
        .as_ref()
        .map(|value| value.to_string())
        .into_iter()
        .collect();
    let id = request.as_ref().and_then(response_id);
    world.stdout = drive(world, |harness| {
        harness.event(Event::Response {
            id,
            command: "resume_session".into(),
            success: true,
            data: Some(json!({
                "status": "resumed",
                "session": "cli:same-execution-dir",
                "sessionKey": "cli:same-execution-dir",
                "messageCount": 3
            })),
            error: None,
        });
        harness.full_frame()
    });
    let follow_up_commands = drain_commands(world);
    world.tui_last_commands.extend(follow_up_commands);
}

#[then(
    expr = "the TUI requests that exact session and accepts status {string} without foreign-folder choices"
)]
fn then_same_execution_directory_resumes(world: &mut TuiWorld, expected_status: String) {
    assert_eq!(expected_status, "resumed", "approved safe-resume status");
    assert_eq!(
        resume_key(&world.tui_last_commands).as_deref(),
        Some("cli:same-execution-dir"),
        "the row carrying the current canonical execution directory must request its exact opaque key"
    );
    assert!(
        world.stdout.contains("Resumed session"),
        "a correlated status=resumed response for the same execution directory must complete without a foreign-folder decision; frame:\n{}",
        world.stdout
    );
    assert!(
        !world.stdout.contains("Open original")
            && !world.stdout.contains("Fork here")
            && !world.stdout.contains("Locate"),
        "the same execution directory must not show foreign-folder choices; frame:\n{}",
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
    assert_eq!(
        resume_key(&world.tui_last_commands).as_deref(),
        Some(expected_key.as_str()),
        "exact opaque key must remain a global direct lookup; commands={:?}",
        world.tui_last_commands
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

const CURRENT_MARKER: &str = "current conversation must survive decision";

fn decision_data(kind: &str) -> Value {
    let (session_key, choices) = match kind {
        "foreign" => (
            "cli:foreign",
            vec!["open_original", "fork_current", "cancel"],
        ),
        "legacy" => (
            "cli:legacy",
            vec!["associate_current", "fork_current", "cancel"],
        ),
        "missing" => ("cli:missing", vec!["locate", "fork_current", "cancel"]),
        _ => panic!("unsupported affirmative decision fixture: {kind}"),
    };
    json!({
        "status": "decision_required",
        "session": session_key,
        "decisionId": format!("opaque-{kind}-decision"),
        "choices": choices,
        "activeSessionKey": "cli:current"
    })
}

fn open_decision(world: &mut TuiWorld, kind: &str) -> String {
    init_harness(world);
    let target = decision_data(kind)["session"]
        .as_str()
        .expect("decision fixture session")
        .to_owned();
    drive(world, |harness| {
        harness.add_user_message(CURRENT_MARKER);
        harness.submit(&format!("/resume {target}"));
    });
    let commands = drain_commands(world);
    let request = commands
        .iter()
        .filter_map(|line| command_json(line))
        .find(|value| command_type(value) == Some("resume_session"))
        .unwrap_or_else(|| panic!("exact resume must issue resume_session; commands={commands:?}"));
    let id = response_id(&request);
    drive(world, |harness| {
        harness.event(Event::Response {
            id,
            command: "resume_session".into(),
            success: true,
            data: Some(decision_data(kind)),
            error: None,
        });
        harness.full_frame()
    })
}

fn resolved_action(commands: &[String]) -> Option<String> {
    commands
        .iter()
        .filter_map(|line| command_json(line))
        .find(|value| command_type(value) == Some("resolve_resume"))
        .and_then(|value| {
            value
                .get("action")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
}

fn activate_decision_by_keyboard(kind: &str, label: &str) -> Option<String> {
    for tabs in 0..5 {
        let mut candidate = TuiWorld::default();
        candidate.stdout = open_decision(&mut candidate, kind);
        if !candidate.stdout.contains(label) {
            return None;
        }
        for _ in 0..tabs {
            drive(&mut candidate, |harness| {
                harness.press(Key::Tab);
            });
        }
        drive(&mut candidate, |harness| {
            harness.press(Key::Enter);
        });
        let commands = drain_commands(&mut candidate);
        if let Some(action) = resolved_action(&commands) {
            return Some(action);
        }
    }
    None
}

fn activate_decision_in_world(
    world: &mut TuiWorld,
    kind: &str,
    expected_action: &str,
) -> Option<Value> {
    for tabs in 0..5 {
        world.stdout = open_decision(world, kind);
        for _ in 0..tabs {
            drive(world, |harness| {
                harness.press(Key::Tab);
            });
        }
        drive(world, |harness| {
            harness.press(Key::Enter);
        });
        let commands = drain_commands(world);
        if let Some(request) = commands
            .iter()
            .filter_map(|line| command_json(line))
            .find(|value| {
                command_type(value) == Some("resolve_resume")
                    && value.get("action").and_then(Value::as_str) == Some(expected_action)
            })
        {
            return Some(request);
        }
    }
    None
}

fn click_decision_label(kind: &str, label: &str) -> Option<String> {
    let mut candidate = TuiWorld::default();
    let frame = open_decision(&mut candidate, kind);
    let (row, line, byte_col) = frame
        .lines()
        .enumerate()
        .find_map(|(row, line)| line.find(label).map(|col| (row, line, col)))?;
    let col = line[..byte_col].chars().count();
    drive(&mut candidate, |harness| {
        harness.press(Key::MousePress(col as u16, row as u16));
        harness.press(Key::MouseRelease(col as u16, row as u16));
    });
    resolved_action(&drain_commands(&mut candidate))
}

#[given("a foreign-folder resume decision in the TUI")]
fn given_foreign_decision(world: &mut TuiWorld) {
    world.stdout = open_decision(world, "foreign");
}

#[given("a legacy resume decision in the TUI")]
fn given_legacy_decision(world: &mut TuiWorld) {
    world.stdout = open_decision(world, "legacy");
}

#[given("a missing-folder resume decision in the TUI")]
fn given_missing_decision(world: &mut TuiWorld) {
    world.stdout = open_decision(world, "missing");
}

#[when("I activate each foreign decision choice with the keyboard")]
fn when_activate_foreign_keyboard(world: &mut TuiWorld) {
    world.tui_last_commands = ["Open original", "Fork here", "Cancel"]
        .into_iter()
        .map(|label| activate_decision_by_keyboard("foreign", label).unwrap_or_default())
        .collect();
}

#[when("I click each foreign decision choice")]
fn when_click_foreign_choices(world: &mut TuiWorld) {
    world.tui_last_commands = ["Open original", "Fork here", "Cancel"]
        .into_iter()
        .map(|label| click_decision_label("foreign", label).unwrap_or_default())
        .collect();
}

#[then("Open original Fork here and Cancel each send their allowlisted action")]
fn then_foreign_actions_sent(world: &mut TuiWorld) {
    assert_eq!(
        world.tui_last_commands,
        ["open_original", "fork_current", "cancel"],
        "each visible decision control must send its affirmative allowlisted resolve_resume action"
    );
}

#[then("the decision modal offers Associate Fork here and Cancel")]
fn then_legacy_controls(world: &mut TuiWorld) {
    for label in ["Associate", "Fork here", "Cancel"] {
        assert!(
            world.stdout.contains(label),
            "legacy decision must visibly offer {label}; frame:\n{}",
            world.stdout
        );
    }
    assert!(
        !world.stdout.contains("Open original") && !world.stdout.contains("Locate"),
        "legacy decision must render only its affirmative allowlist; frame:\n{}",
        world.stdout
    );
}

#[then("the decision modal offers Locate Fork here and Cancel")]
fn then_missing_controls(world: &mut TuiWorld) {
    for label in ["Locate", "Fork here", "Cancel"] {
        assert!(
            world.stdout.contains(label),
            "missing-folder decision must visibly offer {label}; frame:\n{}",
            world.stdout
        );
    }
    assert!(
        !world.stdout.contains("Open original") && !world.stdout.contains("Associate"),
        "missing-folder decision must render only its affirmative allowlist; frame:\n{}",
        world.stdout
    );
}

#[when("I cancel the decision with the keyboard")]
fn when_cancel_decision_keyboard(world: &mut TuiWorld) {
    let request = activate_decision_in_world(world, "foreign", "cancel");
    world.tui_last_commands = request
        .as_ref()
        .and_then(|value| value.get("action").and_then(Value::as_str))
        .map(str::to_owned)
        .into_iter()
        .collect();
    world.stdout = drive(world, TuiHarness::full_frame);
}

#[then("the current conversation remains unchanged")]
fn then_current_conversation_unchanged(world: &mut TuiWorld) {
    assert_eq!(world.tui_last_commands, ["cancel"]);
    assert!(
        world.stdout.contains(CURRENT_MARKER),
        "planning or cancellation must not replace current conversation history; frame:\n{}",
        world.stdout
    );
}

#[when("I choose Open original and the target becomes ready")]
fn when_open_original_ready(world: &mut TuiWorld) {
    let request = activate_decision_in_world(world, "foreign", "open_original");
    world.tui_last_commands = request
        .as_ref()
        .and_then(|value| value.get("action").and_then(Value::as_str))
        .map(str::to_owned)
        .into_iter()
        .collect();
    let id = request.as_ref().and_then(response_id);
    world.stdout = drive(world, |harness| {
        harness.event(Event::Response {
            id,
            command: "resolve_resume".into(),
            success: true,
            data: Some(json!({
                "status": "resumed",
                "session": "cli:foreign",
                "sessionKey": "cli:foreign",
                "messageCount": 3
            })),
            error: None,
        });
        harness.full_frame()
    });
}

#[then("the TUI enters the target conversation")]
fn then_enters_target_conversation(world: &mut TuiWorld) {
    assert_eq!(world.tui_last_commands, ["open_original"]);
    assert!(
        world.stdout.contains("Resumed session") && !world.stdout.contains(CURRENT_MARKER),
        "successful Open original readiness must enter the target conversation; frame:\n{}",
        world.stdout
    );
}
