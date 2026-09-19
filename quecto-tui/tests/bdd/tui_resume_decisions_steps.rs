//! Steps for `tui_resume_decisions.feature` (#2011): the production `/resume`
//! submit path, the typed `resume_session` answer mapping and the decision
//! dialog, driven through the headless harness. The harness's answers are
//! the wire shapes the UDS protocol documents.

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
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

// ─── Review R1: real terminal sizes, fail-closed answers, peers, Ctrl-C ─────

const LONG_FOLDER: &str = "/home/user/Documents/github/some-organisation/\
    a-rather-long-project-name/packages/frontend-application";
const INVISIBLE: [char; 6] = [
    '\u{202e}', '\u{202c}', '\u{2066}', '\u{200b}', '\u{200d}', '\u{feff}',
];

#[given(expr = "a fresh TUI app harness on a {int} by {int} terminal")]
fn given_sized_harness(world: &mut TuiWorld, columns: usize, rows: usize) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let h = rt.block_on(TuiHarness::sized(columns, rows));
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
    world.tui_last_commands.clear();
}

#[when(
    expr = "the harness answers the resume with a {string} decision for a session saved in a long folder path"
)]
fn when_long_folder_decision(world: &mut TuiWorld, kind: String) {
    let actions = match kind.as_str() {
        "legacy_unscoped" => "associate,cancel",
        _ => "open_original,fork_current,cancel",
    };
    let mut data = decision(&kind, actions, "cancel");
    data["session"] = serde_json::json!("chat-1750000000-ab");
    data["executionPath"] = serde_json::json!(LONG_FOLDER);
    data["actions"][0]["reason"] = serde_json::json!(
        "explicit association of a legacy session with a folder is not available yet; \
         start a new session — the old transcript stays in place and visible under All Folders"
    );
    let id = pending_resume_id(world);
    answer(world, &id, data);
}

/// The dialog's rows, top to bottom: the text between the box's own two
/// borders (the pane divider to their left is not one of them).
fn dialog_rows(world: &mut TuiWorld) -> Vec<String> {
    let frame = drive(world, TuiHarness::full_frame);
    let inside = |line: &str| {
        let end = line.rfind('│')?;
        let start = line[..end].rfind('│')? + '│'.len_utf8();
        let row = line[start..end].trim();
        (!row.starts_with('─')).then(|| row.to_string())
    };
    let boxed = frame
        .lines()
        .skip_while(|line| !line.contains('┌'))
        .take_while(|line| !line.contains('└'));
    boxed.filter_map(inside).collect()
}

#[then(expr = "the decision dialog box is whole on the {int} by {int} terminal")]
fn then_box_whole(world: &mut TuiWorld, columns: usize, rows: usize) {
    let frame = drive(world, TuiHarness::full_frame);
    let lines: Vec<&str> = frame.lines().collect();
    assert!(lines.len() <= rows, "{} rows: {frame}", lines.len());
    for line in &lines {
        let width = quecto_tui::components::utils::visible_width(line);
        assert!(width <= columns, "{width} columns: {line:?}");
    }
    let top = lines.iter().position(|l| l.contains('┌'));
    let bottom = lines.iter().position(|l| l.contains('└'));
    assert!(
        top.expect("top border") < bottom.expect("bottom border"),
        "{frame}"
    );
    // The footer is inside the box, whole: never a clipped or elided line.
    let body = dialog_rows(world);
    let footer = body.last().expect("rows");
    let ends_the_box = footer.contains("Esc cancel") || footer.contains("restored or linked");
    assert!(ends_the_box && !footer.ends_with('…'), "{frame}");
    if columns >= 80 {
        let said = body.join(" ");
        assert!(said.contains("nothing is restored or linked"), "{frame}");
    }
}

/// The columns a dialog row has, from its top border.
fn dialog_content_width(world: &mut TuiWorld) -> usize {
    let frame = drive(world, TuiHarness::full_frame);
    let border = frame.lines().find(|line| line.contains('┌'));
    border
        .expect("border")
        .matches('─')
        .count()
        .saturating_sub(2)
}

#[then("the decision dialog shows both ends of the recorded folder")]
fn then_folder_ends(world: &mut TuiWorld) {
    let body = dialog_rows(world).concat();
    let (head, tail) = (&LONG_FOLDER[..6], &LONG_FOLDER[LONG_FOLDER.len() - 8..]);
    let from = body.find(head);
    let from = from.unwrap_or_else(|| panic!("the root {head:?}: {body}"));
    assert!(
        body[from..].contains(tail),
        "the last component {tail:?}: {body}"
    );
}

#[then("every unavailable decision row carries its mark and Cancel carries none")]
fn then_every_row_marked(world: &mut TuiWorld) {
    let rows = dialog_rows(world);
    // The offers are the block after the first blank row.
    let offers: Vec<&String> = rows
        .iter()
        .skip_while(|row| !row.is_empty())
        .skip(1)
        .take_while(|row| !row.is_empty())
        .collect();
    let is_cancel = |row: &&String| row.trim_start_matches('→').trim() == "Cancel";
    let cancel: Vec<&String> = offers.iter().copied().filter(is_cancel).collect();
    let unavailable: Vec<&String> = offers
        .iter()
        .copied()
        .filter(|row| !is_cancel(row))
        .collect();
    assert_eq!(cancel.len(), 1, "one unmarked Cancel row: {rows:#?}");
    assert!(!unavailable.is_empty(), "{rows:#?}");
    for row in unavailable {
        let marked = row.contains("unavailable") || row.contains("(n/a)") || row.contains('✗');
        assert!(marked, "{row:?} in {rows:#?}");
    }
}

#[then("the decision dialog explains the unavailable action under the cursor in whole words")]
fn then_reason_in_whole_words(world: &mut TuiWorld) {
    let width = dialog_content_width(world);
    let rows = dialog_rows(world);
    let from = rows
        .iter()
        .position(|row| row.starts_with("Unavailable:") || row.starts_with("explicit"));
    let from = from.unwrap_or_else(|| panic!("the reason is shown: {rows:#?}"));
    let reason: Vec<&String> = rows[from..]
        .iter()
        .take_while(|row| !row.is_empty())
        .collect();
    assert!((1..=4).contains(&reason.len()), "bounded: {reason:#?}");
    let said = "Unavailable: explicit association of a legacy session with a folder is not \
        available yet; start a new session — the old transcript stays in place and visible \
        under All Folders";
    let words: Vec<&str> = said.split_whitespace().collect();
    for row in reason {
        for word in row.trim_end_matches('…').split_whitespace() {
            // Only a word wider than the dialog itself may be broken.
            let too_wide = |w: &&str| w.chars().count() > width && w.contains(word);
            let whole = words.contains(&word) || words.iter().any(too_wide);
            assert!(whole, "{word:?} is a broken word in {row:?}");
        }
    }
}

#[when("I press Ctrl-C in the decision dialog")]
fn when_ctrl_c(world: &mut TuiWorld) {
    let _ = drain(world);
    drive(world, |h| {
        h.press(Key::Ctrl('c'));
    });
}

#[given(expr = "the TUI shows the session {string}")]
fn given_shows_session(world: &mut TuiWorld, key: String) {
    let state = serde_json::json!({
        "type": "response", "id": "s2011f1-state", "command": "get_state", "success": true,
        "data": {"sessionKey": key},
    });
    drive(world, |h| {
        h.event_line(&state.to_string());
    });
    then_still_shows(world, key);
}

#[when(expr = "the harness answers the resume as a success with outcome {string} for {string}")]
fn when_unreadable_success(world: &mut TuiWorld, outcome: String, key: String) {
    let id = pending_resume_id(world);
    let _ = drain(world);
    let line = serde_json::json!({
        "type": "response", "id": id, "command": "resume_session", "success": true,
        "data": {"outcome": outcome, "session": key, "sessionKey": key, "messageCount": 3},
    });
    drive(world, |h| {
        h.event_line(&line.to_string());
    });
}

#[then(expr = "the TUI still shows the session {string}")]
fn then_still_shows(world: &mut TuiWorld, key: String) {
    let shown = drive(world, |h| h.session_key());
    assert_eq!(shown.as_deref(), Some(key.as_str()));
}

#[then("the TUI never reports a resumed session")]
fn then_never_resumed(world: &mut TuiWorld) {
    let notes = drive(world, |h| h.notification_messages());
    assert!(
        !notes.iter().any(|note| note.contains("Resumed")),
        "{notes:?}"
    );
}

#[then("the answer made the TUI send nothing")]
fn then_answer_sent_nothing(world: &mut TuiWorld) {
    let commands = drain(world);
    assert!(commands.is_empty(), "{commands:?}");
}

#[when(expr = "another client's resume is refused with code {string}")]
fn when_foreign_refusal(world: &mut TuiWorld, code: String) {
    let line = serde_json::json!({
        "type": "response", "id": "tab9:resume-peer", "command": "resume_session",
        "success": false, "error": "session not found: theirs",
        "data": {"outcome": "refused", "code": code},
    });
    drive(world, |h| {
        h.event_line(&line.to_string());
    });
}

#[then("the TUI reports nothing")]
fn then_reports_nothing(world: &mut TuiWorld) {
    let notes = drive(world, |h| h.notification_messages());
    assert!(notes.is_empty(), "{notes:?}");
}

#[when("the harness rejects the resume line with an uncorrelated parse error")]
fn when_parse_error(world: &mut TuiWorld) {
    let line = serde_json::json!({
        "type": "response", "command": "parse_error", "success": false,
        "error": "unknown resume action at line 1 column 71",
    });
    drive(world, |h| {
        h.event_line(&line.to_string());
    });
}

#[then("no resume request is left in flight")]
fn then_nothing_in_flight(world: &mut TuiWorld) {
    let pending = drive(world, |h| h.pending_resume_request_id());
    assert_eq!(pending, None);
}

#[when("the harness answers the resume with a decision carrying bidi and zero-width characters")]
fn when_invisible_decision(world: &mut TuiWorld) {
    let soup = String::from_iter(INVISIBLE);
    let id = pending_resume_id(world);
    let mut data = decision(
        "cross_folder",
        "open_original,fork_current,cancel",
        "cancel",
    );
    data["session"] = serde_json::json!(format!("cli:{soup}foreign"));
    data["executionPath"] = serde_json::json!(format!("/srv/{soup}gpj.exe"));
    data["actions"][0]["reason"] = serde_json::json!(format!("later{soup}"));
    answer(world, &id, data);
}

#[then("the rendered frame carries no bidi or zero-width character")]
fn then_no_invisible(world: &mut TuiWorld) {
    let frame = drive(world, TuiHarness::full_frame_raw);
    for ch in INVISIBLE {
        assert!(!frame.contains(ch), "U+{:04X}", ch as u32);
    }
    assert!(frame.contains("gpj.exe"), "{frame}");
}
