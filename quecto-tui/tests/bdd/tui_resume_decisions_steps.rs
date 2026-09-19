//! Steps for `tui_resume_decisions.feature` (#2011): the production `/resume`
//! submit path, the typed `resume_session` answer mapping and the decision
//! dialog, driven through the headless harness. The harness's answers are
//! the wire shapes the UDS protocol documents.

use crate::TuiWorld;
use cucumber::{then, when};
use quecto_tui::shell::app::tui_harness::TuiHarness;
use quecto_tui::shell::keys::Key;

const DECIDED_VERSION: &str = "h1-0123456789abcdef";

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

fn drain(world: &mut TuiWorld) -> Vec<String> {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let h = &mut world.tui_parity.as_mut().expect("harness").0;
    handle.block_on(h.drain_commands())
}

fn resume_requests(commands: &[String]) -> Vec<serde_json::Value> {
    commands
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|command| command["type"] == "resume_session")
        .collect()
}

/// The id of the `resume_session` request the submit step captured.
fn pending_resume_id(world: &TuiWorld) -> String {
    let requests = resume_requests(&world.tui_last_commands);
    assert_eq!(requests.len(), 1, "one resume request: {requests:?}");
    requests[0]["id"].as_str().expect("request id").to_string()
}

fn answer(world: &mut TuiWorld, id: &str, data: serde_json::Value) {
    let line = serde_json::json!({
        "type": "response",
        "id": id,
        "command": "resume_session",
        "success": false,
        "data": data,
        "error": "session resume unavailable: …",
    })
    .to_string();
    drive(world, |h| {
        h.event_line(&line);
    });
}

fn decision(kind: &str, actions: &str, available: &str) -> serde_json::Value {
    let available: Vec<&str> = available.split(',').collect();
    let actions: Vec<_> = actions
        .split(',')
        .map(|action| {
            let is_available = available.contains(&action);
            serde_json::json!({
                "action": action,
                "available": is_available,
                "reason": if is_available {
                    serde_json::Value::Null
                } else {
                    serde_json::json!(format!("{action} is not delivered yet"))
                },
            })
        })
        .collect();
    serde_json::json!({
        "outcome": "decision",
        "code": "decision_required",
        "session": "cli:foreign",
        "sessionKey": "cli:foreign",
        "kind": kind,
        "homeVersion": DECIDED_VERSION,
        "executionPath": "/work/elsewhere",
        "detail": null,
        "actions": actions,
    })
}

#[then(expr = "one resume request is sent for {string} with no action and no version")]
fn then_exact_request(world: &mut TuiWorld, key: String) {
    let requests = resume_requests(&world.tui_last_commands);
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0]["session"], key);
    assert!(requests[0].get("action").is_none(), "{requests:?}");
    assert!(
        requests[0].get("expectedHomeVersion").is_none(),
        "{requests:?}"
    );
}

#[when(
    expr = "the harness answers the resume with a {string} decision offering {string} where {string} is available"
)]
fn when_decision(world: &mut TuiWorld, kind: String, actions: String, available: String) {
    let id = pending_resume_id(world);
    answer(world, &id, decision(&kind, &actions, &available));
}

#[when(expr = "the harness refuses the resume with code {string}")]
fn when_refused(world: &mut TuiWorld, code: String) {
    let id = pending_resume_id(world);
    answer(
        world,
        &id,
        serde_json::json!({"outcome": "refused", "code": code}),
    );
}

#[when("another client's resume is answered with a decision")]
fn when_foreign_decision(world: &mut TuiWorld) {
    answer(
        world,
        "other-tab:resume-7",
        decision(
            "cross_folder",
            "open_original,fork_current,cancel",
            "cancel",
        ),
    );
}

#[when("the harness answers the resume with a decision carrying terminal control characters")]
fn when_hostile_decision(world: &mut TuiWorld) {
    let id = pending_resume_id(world);
    let mut data = decision("home_unknown", "locate,fork_current,cancel", "cancel");
    data["session"] = serde_json::json!("cli:foreign\u{1b}[2J");
    data["executionPath"] = serde_json::json!("/work/\u{1b}]0;owned\u{7}");
    data["detail"] = serde_json::json!("bad\u{1b}[31mrecord");
    data["actions"][0]["reason"] = serde_json::json!("later\u{1b}[5m");
    answer(world, &id, data);
}

#[when("I press Escape in the decision dialog")]
fn when_escape(world: &mut TuiWorld) {
    drain(world);
    drive(world, |h| {
        h.press(Key::Escape);
    });
}

#[when(expr = "I choose decision row {int}")]
fn when_choose_row(world: &mut TuiWorld, row: usize) {
    drain(world);
    drive(world, |h| {
        for _ in 1..row {
            h.press(Key::Down);
        }
        h.press(Key::Enter);
    });
}

#[then(expr = "the decision dialog is titled {string}")]
fn then_titled(world: &mut TuiWorld, title: String) {
    let frame = drive(world, TuiHarness::full_frame);
    assert!(frame.contains(&title), "{frame}");
    assert!(frame.contains("nothing is restored or linked"), "{frame}");
}

#[then(expr = "the decision dialog lists {string} in order")]
fn then_lists(world: &mut TuiWorld, labels: String) {
    let frame = drive(world, TuiHarness::full_frame);
    let mut from = 0;
    for label in labels.split(',') {
        let at = frame[from..]
            .find(label)
            .unwrap_or_else(|| panic!("{label:?} missing or out of order: {frame}"));
        from += at + label.len();
    }
}

#[then("every action except Cancel is marked unavailable")]
fn then_marked_unavailable(world: &mut TuiWorld) {
    let frame = drive(world, TuiHarness::full_frame);
    let rows: Vec<&str> = frame
        .lines()
        .filter(|line| {
            [
                "Open original",
                "Fork into",
                "Locate folder",
                "Associate with",
            ]
            .iter()
            .any(|label| line.contains(label))
        })
        .collect();
    assert!(!rows.is_empty(), "{frame}");
    for row in rows {
        assert!(row.contains("unavailable"), "{row:?} in {frame}");
    }
    let cancel = frame
        .lines()
        .find(|line| line.contains("Cancel") && !line.contains("Esc cancel"))
        .expect("cancel row");
    assert!(!cancel.contains("unavailable"), "{cancel:?}");
}

#[then("the decision dialog is closed")]
fn then_closed(world: &mut TuiWorld) {
    let frame = drive(world, TuiHarness::full_frame);
    assert!(!frame.contains("nothing is restored or linked"), "{frame}");
}

#[then("the decision dialog sent no command")]
fn then_sent_nothing(world: &mut TuiWorld) {
    let commands = drain(world);
    assert!(commands.is_empty(), "{commands:?}");
}

#[then(expr = "the TUI explains {string}")]
fn then_explains(world: &mut TuiWorld, text: String) {
    let messages = drive(world, |h| h.notification_messages());
    assert!(
        messages.iter().any(|message| message.contains(&text)),
        "{text:?} not among {messages:?}"
    );
}

#[then(
    expr = "one resume request is sent for {string} with action {string} and the decided version"
)]
fn then_action_request(world: &mut TuiWorld, key: String, action: String) {
    let requests = resume_requests(&drain(world));
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0]["session"], key);
    assert_eq!(requests[0]["action"], action);
    assert_eq!(requests[0]["expectedHomeVersion"], DECIDED_VERSION);
}

#[then("the rendered frame carries no raw control sequence from the decision")]
fn then_no_raw_controls(world: &mut TuiWorld) {
    // The raw frame: theme styling is present, the decision's own sequences
    // (clear screen, set title, bell, blink) are not.
    let raw = drive(world, TuiHarness::full_frame_raw);
    for hostile in ["\u{1b}[2J", "\u{1b}]0;", "\u{7}", "\u{1b}[5m"] {
        assert!(!raw.contains(hostile), "{hostile:?} in {raw:?}");
    }
    let frame = drive(world, TuiHarness::full_frame);
    assert!(frame.contains("cli:foreign"), "{frame}");
}
