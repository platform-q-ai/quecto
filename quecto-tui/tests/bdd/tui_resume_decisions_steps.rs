//! Steps for `tui_resume_decisions.feature` (#2011): the production `/resume`
//! submit path, the typed `resume_session` answer mapping and the decision
//! dialog, driven through the headless harness. The harness's answers are
//! the wire shapes the UDS protocol documents.

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::shell::app::tui_harness::TuiHarness;
use quecto_tui::shell::keys::Key;

const DECIDED_VERSION: &str = "h1-0123456789abcdef";
/// What only the decision dialog's footer says.
const FOOTER_TAIL: &str = "your current session is untouched";
/// The harness's own reasons (`dto/resume_decision.rs::unavailable_reason`).
const OPEN_REASON: &str =
    "Not available yet. To continue this session, start quecto in that folder.";
const ASSOCIATE_REASON: &str = "Not available yet. This session predates folder tracking; \
    it stays listed under All Folders.";

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
    assert!(frame.contains(FOOTER_TAIL), "{frame}");
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
            ["Open original", "Copy into", "Locate folder", "Attach to"]
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
    assert!(!frame.contains(FOOTER_TAIL), "{frame}");
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
    data["actions"][0]["reason"] = serde_json::json!(shipped_reason(&kind));
    let id = pending_resume_id(world);
    answer(world, &id, data);
}

/// The reason the harness really ships for the first offer of `kind`.
fn shipped_reason(kind: &str) -> &'static str {
    match kind {
        "legacy_unscoped" => ASSOCIATE_REASON,
        _ => OPEN_REASON,
    }
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
    let ends_the_box = footer.contains("Esc cancel") || footer.contains("session is untouched");
    assert!(ends_the_box && !footer.ends_with('…'), "{frame}");
    if columns >= 80 {
        let said = body.join(" ");
        assert!(said.contains(FOOTER_TAIL), "{frame}");
    }
    // Review R2-T3: the box spans the TERMINAL, not the body pane beside the
    // agents pane — a 40-column terminal gives it 36 columns, not 10.
    let wanted = columns.saturating_sub(4).min(88);
    assert_eq!(dialog_content_width(world) + 4, wanted, "{frame}");
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

/// The offers: the block after the first blank row, up to the next blank row
/// or to the reason (a small terminal sheds the blank between the two).
fn offer_rows(rows: &[String]) -> Vec<&String> {
    rows.iter()
        .skip_while(|row| !row.is_empty())
        .skip(1)
        .take_while(|row| !row.is_empty() && !row.starts_with("Not available"))
        .collect()
}

#[then("every unavailable decision row carries its mark and Cancel carries none")]
fn then_every_row_marked(world: &mut TuiWorld) {
    let rows = dialog_rows(world);
    let offers = offer_rows(&rows);
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

/// Review R2-T1: every action is unavailable in this slice, so the reason is
/// the dialog's content — it is on screen in FULL, whole words, no ellipsis.
#[then("the decision dialog shows the whole reason of the unavailable action under the cursor")]
fn then_whole_reason(world: &mut TuiWorld) {
    let rows = dialog_rows(world);
    let from = rows.iter().position(|row| row.starts_with("Not available"));
    let from = from.unwrap_or_else(|| panic!("the reason is shown: {rows:#?}"));
    let reason: Vec<&str> = rows[from..]
        .iter()
        .take_while(|row| !row.is_empty() && !row.contains("Esc cancel"))
        .map(String::as_str)
        .collect();
    let said = reason.join(" ");
    assert!(
        [OPEN_REASON, ASSOCIATE_REASON].contains(&said.as_str()),
        "the whole reason, word for word: {rows:#?}"
    );
    assert!(!said.contains('…'), "nothing is cut: {rows:#?}");
}

/// Review R2-T3: legible, not merely unclipped — the long title in whole
/// words, and every offer a complete label (long or short form, with a whole
/// mark) or a cut that says so.
#[then("the decision dialog title and every action label read whole")]
fn then_title_and_labels_whole(world: &mut TuiWorld) {
    let rows = dialog_rows(world);
    let said = rows.join(" ");
    let titled = [
        "This session belongs to another folder",
        "This session was saved before quecto tracked folders",
    ];
    assert!(titled.iter().any(|title| said.contains(title)), "{rows:#?}");
    let names = [
        "Open original folder",
        "Open original",
        "Copy into this folder as a new session",
        "Copy here",
        "Attach to a folder",
        "Attach",
    ];
    for row in offer_rows(&rows) {
        let label = row.trim_start_matches('→').trim();
        let whole = label == "Cancel"
            || names.iter().any(|name| {
                [
                    format!("{name} — unavailable"),
                    format!("{name} (n/a)"),
                    format!("✗ {name}"),
                ]
                .contains(&label.to_string())
            });
        assert!(whole || label.ends_with('…'), "{label:?} in {rows:#?}");
        assert!(
            !label.ends_with('…'),
            "no label is cut at this size: {rows:#?}"
        );
    }
}

#[then(expr = "the decision dialog shows {string}")]
fn then_dialog_shows(world: &mut TuiWorld, text: String) {
    let rows = dialog_rows(world).join("\n");
    assert!(rows.contains(&text), "{text:?} not in {rows}");
}

#[when(expr = "the harness answers the resume with a {string} decision whose detail is {string}")]
fn when_decision_with_detail(world: &mut TuiWorld, kind: String, detail: String) {
    let mut data = decision(&kind, "locate,fork_current,cancel", "cancel");
    data["detail"] = serde_json::json!(detail);
    let id = pending_resume_id(world);
    answer(world, &id, data);
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
            "homeVersion": DECIDED_VERSION, "executionPath": "/work/elsewhere",
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

#[then("the resume request is still in flight")]
fn then_still_in_flight(world: &mut TuiWorld) {
    let pending = drive(world, |h| h.pending_resume_request_id());
    assert_eq!(pending, Some(pending_resume_id(world)));
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
