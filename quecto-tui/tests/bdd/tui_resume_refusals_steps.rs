//! Steps for `tui_resume_refusals.feature` (#2011, #2045): the production
//! `/resume` submit path, the typed `resume_session` answer mapping and the
//! plain notice of a refused resume, driven through the headless harness. The
//! harness's answers are the wire shapes the UDS protocol documents.

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::shell::app::tui_harness::TuiHarness;
use quecto_tui::shell::keys::Key;

const LISTED_VERSION: &str = "h1-0123456789abcdef";
/// What only the open notice's footer says.
const FOOTER: &str = "Enter or Esc to close";
const LONG_FOLDER: &str = "/home/user/Documents/github/some-organisation/\
    a-rather-long-project-name/packages/frontend-application";
const INVISIBLE: [char; 6] = [
    '\u{202e}', '\u{202c}', '\u{2066}', '\u{200b}', '\u{200d}', '\u{feff}',
];

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
        "type": "response", "id": id, "command": "resume_session", "success": false,
        "data": data, "error": "session resume unavailable: …",
    })
    .to_string();
    drive(world, |h| {
        h.event_line(&line);
    });
}

/// The refusal the harness documents for a session whose folder is `folder`.
fn refusal(kind: &str, code: &str, folder: &str) -> serde_json::Value {
    serde_json::json!({
        "outcome": "refused", "code": code, "kind": kind,
        "session": "cli:foreign", "sessionKey": "cli:foreign",
        "executionPath": folder, "detail": null,
        "command": format!("cd '{folder}' && quecto-tui"),
        "resume": "/resume cli:foreign",
    })
}

/// The notice's rows, top to bottom: the text between the box's own borders.
fn notice_rows(world: &mut TuiWorld) -> Vec<String> {
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

#[when(expr = "the harness refuses the resume as {string} with code {string}")]
fn when_refused_as(world: &mut TuiWorld, kind: String, code: String) {
    let id = pending_resume_id(world);
    let _ = drain(world);
    answer(world, &id, refusal(&kind, &code, "/work/elsewhere"));
}

#[when(expr = "the harness refuses the resume as {string} with detail {string}")]
fn when_refused_with_detail(world: &mut TuiWorld, kind: String, detail: String) {
    let mut data = refusal(&kind, &kind, "/work/elsewhere");
    data["detail"] = serde_json::json!(detail);
    let id = pending_resume_id(world);
    answer(world, &id, data);
}

#[when("the harness refuses the resume for a session saved in a long folder path")]
fn when_refused_long_folder(world: &mut TuiWorld) {
    let id = pending_resume_id(world);
    answer(
        world,
        &id,
        refusal("cross_folder", "belongs_elsewhere", LONG_FOLDER),
    );
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

#[when(expr = "an older harness answers the resume with a {string} decision")]
fn when_older_decision(world: &mut TuiWorld, kind: String) {
    let id = pending_resume_id(world);
    answer(
        world,
        &id,
        serde_json::json!({
            "outcome": "decision", "code": "decision_required", "kind": kind,
            "session": "cli:foreign", "sessionKey": "cli:foreign",
            "homeVersion": LISTED_VERSION, "executionPath": "/work/gone",
            "detail": "No such file or directory",
            "actions": [{"action": "cancel", "available": true, "reason": null}],
        }),
    );
}

#[when("the harness refuses the resume with a command carrying terminal control characters")]
fn when_hostile_command(world: &mut TuiWorld) {
    let mut data = refusal("cross_folder", "belongs_elsewhere", "/work/elsewhere");
    data["command"] = serde_json::json!("cd '/work/else\u{1b}[2J\u{7}where' && quecto-tui");
    data["executionPath"] = serde_json::json!("/work/else\u{1b}[2J\u{7}where");
    data["detail"] = serde_json::json!("bell\u{7}\u{1b}]0;owned\u{7}");
    let id = pending_resume_id(world);
    answer(world, &id, data);
}

#[when("the harness refuses the resume with a folder carrying bidi and zero-width characters")]
fn when_invisible_folder(world: &mut TuiWorld) {
    let hostile: String = INVISIBLE.iter().collect();
    let mut data = refusal("home_missing", "home_missing", "/work/elsewhere");
    data["executionPath"] = serde_json::json!(format!("/work/{hostile}gnp.exe"));
    data["detail"] = serde_json::json!(format!("detail{hostile}"));
    data["command"] = serde_json::Value::Null;
    let id = pending_resume_id(world);
    answer(world, &id, data);
}

#[when("another client's resume is refused as belonging elsewhere")]
fn when_peer_refusal(world: &mut TuiWorld) {
    answer(
        world,
        "tab9:resume-peer",
        refusal("cross_folder", "belongs_elsewhere", "/work/elsewhere"),
    );
}

#[then(expr = "the notice is titled {string}")]
fn then_titled(world: &mut TuiWorld, title: String) {
    let rows = notice_rows(world);
    assert_eq!(
        rows.first().map(String::as_str),
        Some(title.as_str()),
        "{rows:#?}"
    );
    assert!(rows.iter().any(|row| row == FOOTER), "{rows:#?}");
}

#[then("the notice offers nothing to choose")]
fn then_offers_nothing(world: &mut TuiWorld) {
    let shown = notice_rows(world).join("\n");
    for gone in [
        "Open original",
        "Copy into",
        "Locate",
        "Attach",
        "Cancel",
        "unavailable",
        "▸",
        "> ",
    ] {
        assert!(!shown.contains(gone), "{gone:?} in {shown}");
    }
}

#[then(expr = "the notice shows {string}")]
fn then_shows(world: &mut TuiWorld, text: String) {
    let rows = notice_rows(world);
    assert!(rows.join("\n").contains(&text), "{text:?} in {rows:#?}");
}

#[then(expr = "the notice never shows {string}")]
fn then_never_shows(world: &mut TuiWorld, text: String) {
    let rows = notice_rows(world);
    assert!(!rows.is_empty(), "the notice is open");
    assert!(!rows.join("\n").contains(&text), "{text:?} in {rows:#?}");
}

#[then("the notice is closed")]
fn then_closed(world: &mut TuiWorld) {
    let frame = drive(world, TuiHarness::full_frame);
    assert!(!frame.contains(FOOTER), "{frame}");
}

#[then("the notice sent no command")]
fn then_sent_nothing(world: &mut TuiWorld) {
    let commands = drain(world);
    assert!(commands.is_empty(), "{commands:?}");
}

#[when(expr = "I press {word} in the notice")]
fn when_press(world: &mut TuiWorld, key: String) {
    let key = match key.as_str() {
        "Escape" => Key::Escape,
        "Enter" => Key::Enter,
        "Ctrl-C" => Key::Ctrl('c'),
        other => panic!("unknown key {other}"),
    };
    let _ = drain(world);
    drive(world, |h| {
        h.press(key);
    });
}

#[then(expr = "the TUI explains {string}")]
fn then_explains(world: &mut TuiWorld, text: String) {
    let notes = drive(world, |h| h.notification_messages());
    assert!(notes.iter().any(|note| note.contains(&text)), "{notes:?}");
}

#[given(expr = "a fresh TUI app harness on a {int} by {int} terminal")]
fn given_sized_harness(world: &mut TuiWorld, columns: usize, rows: usize) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let h = rt.block_on(TuiHarness::sized(columns, rows));
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
    world.tui_last_commands.clear();
}

#[then(expr = "the notice box is whole on the {int} by {int} terminal")]
fn then_box_whole(world: &mut TuiWorld, columns: usize, rows: usize) {
    let frame = drive(world, TuiHarness::full_frame);
    let lines: Vec<&str> = frame.lines().collect();
    assert!(lines.len() <= rows, "{} lines on {rows} rows", lines.len());
    let top = lines
        .iter()
        .position(|l| l.contains('┌'))
        .expect("top border");
    let bottom = lines
        .iter()
        .position(|l| l.contains('└'))
        .expect("bottom border");
    let footer = lines
        .iter()
        .position(|l| l.contains(FOOTER))
        .expect("footer");
    assert!(top < footer && footer < bottom, "{frame}");
    for line in &lines {
        let width = quecto_tui::components::utils::visible_width(line);
        assert!(width <= columns, "{width} > {columns}: {line:?}");
    }
}

#[then("the notice shows the whole command and the resume step")]
fn then_whole_command(world: &mut TuiWorld) {
    let rows = notice_rows(world);
    // The command is cut at the column and nowhere else: its characters, in
    // order, with the line breaks removed — never an ellipsis.
    let joined = rows.join("");
    let command = format!("cd '{LONG_FOLDER}' && quecto-tui");
    assert!(joined.contains(&command), "{command:?} whole in {rows:#?}");
    assert!(joined.contains("/resume cli:foreign"), "{rows:#?}");
}

#[then("the notice shows both ends of the recorded folder")]
fn then_both_ends(world: &mut TuiWorld) {
    let rows = notice_rows(world);
    let folder_rows: Vec<&String> = rows
        .iter()
        // The title wraps on a narrow terminal; the folder starts at its root.
        .skip_while(|row| !row.starts_with('/'))
        .take_while(|row| !row.starts_with("Open quecto"))
        .collect();
    let folder = folder_rows.iter().map(|r| r.as_str()).collect::<String>();
    assert!(folder.starts_with("/home/user"), "{rows:#?}");
    assert!(folder.ends_with("frontend-application"), "{rows:#?}");
}

#[when(expr = "the harness lists the session {string} titled {string} saved in another folder")]
fn when_listed_elsewhere(world: &mut TuiWorld, key: String, title: String) {
    let request = world
        .tui_last_commands
        .iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|command| command["type"] == "list_sessions")
        .expect("the /resume listing request");
    let line = serde_json::json!({
        "type": "response", "id": request["id"], "command": "list_sessions", "success": true,
        "data": {"sessions": [{
            "title": title, "key": key, "messageCount": 3, "resumeEligible": false,
            "homeVersion": LISTED_VERSION, "executionPath": "/work/elsewhere",
        }]},
    });
    drive(world, |h| {
        h.event_line(&line.to_string());
    });
}

#[then(expr = "the resume list explains the row with {string}")]
fn then_row_explained(world: &mut TuiWorld, hint: String) {
    let frame = drive(world, TuiHarness::full_frame);
    assert!(frame.contains(&hint), "{frame}");
    assert!(!frame.contains("Needs a decision"), "{frame}");
}

#[when("I pick the listed session")]
fn when_pick_listed(world: &mut TuiWorld) {
    drive(world, |h| {
        h.press(Key::Enter);
    });
    world.tui_last_commands = drain(world);
}

#[given(expr = "the TUI shows the session {string}")]
fn given_shows_session(world: &mut TuiWorld, key: String) {
    let state = serde_json::json!({
        "type": "response", "id": "s2045-state", "command": "get_state", "success": true,
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

#[then("the TUI reports nothing")]
fn then_reports_nothing(world: &mut TuiWorld) {
    let notes = drive(world, |h| h.notification_messages());
    assert!(notes.is_empty(), "{notes:?}");
}

#[then("no resume request is left in flight")]
fn then_nothing_in_flight(world: &mut TuiWorld) {
    let pending = drive(world, |h| h.pending_resume_request_id());
    assert_eq!(pending, None);
}

#[then("the rendered frame carries no raw control sequence")]
fn then_no_raw_control(world: &mut TuiWorld) {
    let rows = notice_rows(world).join("\n");
    for bad in ["[2J", "]0;owned", "\u{1b}", "\u{7}"] {
        assert!(!rows.contains(bad), "{bad:?} in {rows:?}");
    }
}

#[then("the rendered frame carries no bidi or zero-width character")]
fn then_no_invisible(world: &mut TuiWorld) {
    let frame = drive(world, TuiHarness::full_frame);
    for ch in INVISIBLE {
        assert!(!frame.contains(ch), "U+{:04X} in {frame:?}", ch as u32);
    }
    assert!(frame.contains(FOOTER), "the notice is open: {frame}");
}
