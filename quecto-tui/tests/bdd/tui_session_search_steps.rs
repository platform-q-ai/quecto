//! Steps for `tui_session_search.feature` (#2010): the production `/resume`
//! picker's search box, driven through the headless harness. The harness's
//! answers are the wire shapes the UDS protocol documents; every assertion is
//! on the commands the TUI really sent and the frame it really drew.

use crate::{TuiParityHarness, TuiWorld};
use cucumber::{given, then, when};
use quecto_tui::shell::app::tui_harness::TuiHarness;
use quecto_tui::shell::keys::Key;

fn drive<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let _guard = handle.enter();
    f(&mut world.tui_parity.as_mut().expect("harness").0)
}

/// Drain what the TUI sent since the last drain into the world's full log.
fn sent(world: &mut TuiWorld) -> Vec<serde_json::Value> {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    let h: &mut TuiParityHarness = world.tui_parity.as_mut().expect("harness");
    let fresh = handle.block_on(h.0.drain_commands());
    world.tui_last_commands.extend(fresh);
    world
        .tui_last_commands
        .iter()
        .map(|line| serde_json::from_str(line).expect("a JSON command"))
        .collect()
}

fn of_type(world: &mut TuiWorld, kind: &str) -> Vec<serde_json::Value> {
    let all = sent(world);
    all.into_iter().filter(|c| c["type"] == kind).collect()
}

fn respond(world: &mut TuiWorld, id: &serde_json::Value, command: &str, data: serde_json::Value) {
    let line = serde_json::json!({
        "type": "response", "id": id, "command": command, "success": true, "data": data,
    });
    drive(world, |h| {
        h.event_line(&line.to_string());
    });
}

fn row(title: &str, key: &str, version: &str, folder: Option<&str>) -> serde_json::Value {
    serde_json::json!({
        "title": title, "key": key, "messageCount": 3, "updatedUnixSecs": 1_700_000_000,
        "resumeEligible": false, "homeVersion": version, "executionPath": folder,
        "homeState": if folder.is_some() { "scoped" } else { "legacy_unscoped" },
        "repositoryLabel": folder.map(|f| f.rsplit('/').next().unwrap_or(f)),
        "matched": ["title"],
    })
}

/// The searches sent and not yet answered by a step, oldest first.
fn unanswered(world: &mut TuiWorld) -> Vec<serde_json::Value> {
    let answered = world.tui_search_answered.clone();
    let searches = of_type(world, "search_session_metadata");
    let open = |s: &serde_json::Value| !answered.contains(&s["id"].as_str().unwrap().to_string());
    searches.into_iter().filter(open).collect()
}

fn answer_in_flight(world: &mut TuiWorld, generation: Option<u64>, rows: Vec<serde_json::Value>) {
    let open = unanswered(world);
    assert_eq!(open.len(), 1, "exactly one search in flight: {open:?}");
    let request = open[0].clone();
    let id = request["id"].as_str().unwrap().to_string();
    world.tui_search_answered.push(id);
    let generation = generation.map_or(request["generation"].clone(), |g| serde_json::json!(g));
    let total = rows.len();
    let data = serde_json::json!({
        "query": request["query"], "scope": request["scope"], "generation": generation,
        "sessions": rows, "totalMatches": total, "searched": 9, "truncated": false,
        "refused": null, "diagnostics": [], "rebuilt": false,
    });
    respond(world, &request["id"], "search_session_metadata", data);
}

#[given(expr = "the resume picker is open on All Folders with {int} listed sessions")]
fn given_picker_open(world: &mut TuiWorld, count: usize) {
    world.tui_last_commands.clear();
    world.tui_search_answered.clear();
    drive(world, |h| {
        h.submit("/resume");
    });
    let rows: Vec<_> = ["LISTED-ONE", "LISTED-TWO", "LISTED-THREE"][..count]
        .iter()
        .enumerate()
        .map(|(n, title)| {
            row(
                title,
                &format!("cli:listed{n}"),
                &format!("h1-00000000000000a{n}"),
                Some("/work/here"),
            )
        })
        .collect();
    for scope in ["local", "global"] {
        let lists = of_type(world, "list_sessions");
        let list = lists.last().expect("a listing request").clone();
        assert_eq!(list["scope"], scope);
        let data = serde_json::json!({"scope": scope, "sessions": rows, "diagnostics": [], "rebuilt": false});
        respond(world, &list["id"], "list_sessions", data);
        if scope == "local" {
            // Sessions ▸ Scope, then Right selects All Folders.
            drive(world, |h| {
                h.press(Key::Tab).press(Key::Right);
            });
        }
    }
    // Scope ▸ Search.
    drive(world, |h| {
        h.press(Key::Tab);
    });
    let frame = drive(world, TuiHarness::full_frame);
    assert!(
        frame.contains("[All Folders]") && frame.contains("LISTED-ONE"),
        "{frame}"
    );
    world.tui_lists_before_typing = of_type(world, "list_sessions").len();
}

#[when(expr = "I type {string} into the resume search box")]
fn when_type(world: &mut TuiWorld, text: String) {
    drive(world, |h| {
        for ch in text.chars() {
            h.press(Key::Char(ch));
        }
    });
}

#[when("I clear the resume search box")]
fn when_clear(world: &mut TuiWorld) {
    drive(world, |h| {
        for _ in 0..64 {
            h.press(Key::Backspace);
        }
    });
}

#[then(expr = "one metadata search is in flight for {string} in scope {string}")]
fn then_one_in_flight(world: &mut TuiWorld, query: String, scope: String) {
    let open = unanswered(world);
    assert_eq!(open.len(), 1, "single flight: {open:?}");
    assert_eq!(
        (open[0]["query"].as_str(), open[0]["scope"].as_str()),
        (Some(query.as_str()), Some(scope.as_str()))
    );
    assert!(
        open[0]["generation"].as_u64().is_some_and(|g| g > 0),
        "{:?}",
        open[0]
    );
}

#[then("no metadata search is in flight")]
fn then_none_in_flight(world: &mut TuiWorld) {
    let open = unanswered(world);
    assert!(open.is_empty(), "{open:?}");
}

#[then("no session list was requested by typing")]
fn then_no_list_by_typing(world: &mut TuiWorld) {
    assert_eq!(
        of_type(world, "list_sessions").len(),
        world.tui_lists_before_typing
    );
}

#[then(expr = "one session list is requested in scope {string}")]
fn then_list_requested(world: &mut TuiWorld, scope: String) {
    let lists = of_type(world, "list_sessions");
    assert_eq!(lists.len(), world.tui_lists_before_typing + 1, "{lists:?}");
    assert_eq!(lists.last().unwrap()["scope"], scope.as_str());
}

#[when("the harness answers the search in flight with no sessions")]
fn when_answer_none(world: &mut TuiWorld) {
    answer_in_flight(world, None, Vec::new());
}

#[when(expr = "the harness answers the search in flight with the session {string}")]
fn when_answer_one(world: &mut TuiWorld, title: String) {
    answer_in_flight(
        world,
        None,
        vec![row(
            &title,
            "cli:answered",
            "h1-00000000000000c1",
            Some("/work/x"),
        )],
    );
}

#[when(
    expr = "the harness answers the search in flight under generation {int} with the session {string}"
)]
fn when_answer_generation(world: &mut TuiWorld, generation: u64, title: String) {
    answer_in_flight(
        world,
        Some(generation),
        vec![row(&title, "cli:gen", "h1-00000000000000c2", None)],
    );
}

#[when("the harness answers the search in flight with a scoped and an unscoped session")]
fn when_answer_two(world: &mut TuiWorld) {
    let rows = vec![
        row(
            "Zebra cache",
            "chat-1700000000-abc",
            "h1-00000000000000b1",
            Some("/work/alpha"),
        ),
        row("An old chat", "cli:legacy", "h1-00000000000000b2", None),
    ];
    answer_in_flight(world, None, rows);
}

#[when(expr = "a search answer with an unknown id arrives with the session {string}")]
#[when(expr = "a search answer for the closed picker arrives with the session {string}")]
fn when_foreign_answer(world: &mut TuiWorld, title: String) {
    let latest = of_type(world, "search_session_metadata").last().cloned();
    let (id, generation) = match latest {
        Some(request) if title == "AFTER-ESCAPE" => {
            (request["id"].clone(), request["generation"].clone())
        }
        Some(request) => (
            serde_json::json!("search-unknown"),
            request["generation"].clone(),
        ),
        None => (serde_json::json!("search-unknown"), serde_json::json!(1)),
    };
    let data = serde_json::json!({
        "generation": generation, "scope": "global", "totalMatches": 1, "diagnostics": [],
        "sessions": [row(&title, "cli:foreign", "h1-00000000000000c3", None)],
    });
    respond(world, &id, "search_session_metadata", data);
}

#[when(expr = "the harness answers every search until {string} with a hostile row")]
fn when_answer_until(world: &mut TuiWorld, query: String) {
    let hostile = "evil\u{1b}[2J \u{202e}title\u{200b}\u{7}";
    for _ in 0..16 {
        let open = unanswered(world);
        let Some(request) = open.first().cloned() else {
            break;
        };
        let folder = "/work/\u{202e}dlof\u{1b}]0;x\u{7}";
        answer_in_flight(
            world,
            None,
            vec![row(
                hostile,
                "cli:evil",
                "h1-00000000000000d1",
                Some(folder),
            )],
        );
        if request["query"] == query.as_str() {
            return;
        }
    }
    panic!("no search for {query:?} was ever sent");
}

#[then(expr = "the latest metadata search asks for {string} literally")]
fn then_latest_query(world: &mut TuiWorld, query: String) {
    let searches = of_type(world, "search_session_metadata");
    assert_eq!(searches.last().expect("a search")["query"], query.as_str());
}

#[when(expr = "the harness fails the search in flight with {string}")]
fn when_fail(world: &mut TuiWorld, error: String) {
    let open = unanswered(world);
    assert_eq!(open.len(), 1);
    world
        .tui_search_answered
        .push(open[0]["id"].as_str().unwrap().to_string());
    let line = serde_json::json!({
        "type": "response", "id": open[0]["id"], "command": "search_session_metadata",
        "success": false, "error": error,
    });
    drive(world, |h| {
        h.event_line(&line.to_string());
    });
}

#[when("I switch the resume picker to Local Folder")]
fn when_switch_local(world: &mut TuiWorld) {
    // Search ◂ Scope, Left selects Local Folder, then back to Search.
    drive(world, |h| {
        h.press(Key::BackTab).press(Key::Left).press(Key::Tab);
    });
}

#[when("I press Down in the resume results")]
fn when_down(world: &mut TuiWorld) {
    drive(world, |h| {
        h.press(Key::Tab).press(Key::Down);
    });
}

#[when("I pick the second searched row")]
fn when_pick_second(world: &mut TuiWorld) {
    drive(world, |h| {
        h.press(Key::Tab).press(Key::Down).press(Key::Enter);
    });
}

#[when("I press Escape in the resume picker")]
fn when_escape(world: &mut TuiWorld) {
    drive(world, |h| {
        h.press(Key::Escape);
    });
}

#[then(expr = "the resume picker shows {string}")]
#[then(expr = "the resume picker details show {string}")]
fn then_shows(world: &mut TuiWorld, text: String) {
    let frame = drive(world, TuiHarness::full_frame);
    assert!(frame.contains(&text), "{text:?} not in {frame}");
}

#[then(expr = "the resume picker shows the row {string} with {string}")]
fn then_shows_row(world: &mut TuiWorld, title: String, folder: String) {
    let frame = drive(world, TuiHarness::full_frame);
    assert!(frame.contains(&title) && frame.contains(&folder), "{frame}");
    assert!(
        !frame.contains("LISTED-ONE"),
        "the listing was replaced: {frame}"
    );
}

#[then(expr = "the resume picker does not show {string}")]
fn then_not_shows(world: &mut TuiWorld, text: String) {
    let frame = drive(world, TuiHarness::full_frame);
    assert!(!frame.contains(&text), "{text:?} in {frame}");
    assert!(
        frame.contains("Resume session"),
        "the picker is still open: {frame}"
    );
}

#[then("the resume picker shows no control, bidi or zero-width character")]
fn then_safe(world: &mut TuiWorld) {
    // The styled frame: what the terminal would really receive.
    let raw = drive(world, TuiHarness::full_frame_raw);
    for hidden in ['\u{7}', '\u{202e}', '\u{200b}'] {
        assert!(
            !raw.contains(hidden),
            "{hidden:?} reached the terminal: {raw:?}"
        );
    }
    for sequence in ["\u{1b}[2J", "\u{1b}]0;"] {
        assert!(!raw.contains(sequence), "{sequence:?} reached the terminal");
    }
}

#[then("the resume picker is closed")]
fn then_closed(world: &mut TuiWorld) {
    let frame = drive(world, TuiHarness::full_frame);
    assert!(
        !frame.contains("Resume session") && !frame.contains("AFTER-ESCAPE"),
        "{frame}"
    );
}

#[then("nothing but metadata searches and lists was ever sent")]
fn then_nothing_else(world: &mut TuiWorld) {
    let all = sent(world);
    let others: Vec<_> = all
        .iter()
        .filter(|c| c["type"] != "search_session_metadata" && c["type"] != "list_sessions")
        .collect();
    assert!(others.is_empty(), "{others:?}");
    assert_eq!(
        of_type(world, "search_session_metadata").len(),
        1,
        "Escape queued nothing"
    );
}

#[then(expr = "one resume request is sent for {string} carrying version {string}")]
fn then_resume_sent(world: &mut TuiWorld, key: String, version: String) {
    let resumes = of_type(world, "resume_session");
    assert_eq!(resumes.len(), 1, "{resumes:?}");
    assert_eq!(resumes[0]["session"], key.as_str());
    assert_eq!(resumes[0]["expectedHomeVersion"], version.as_str());
    assert!(resumes[0].get("action").is_none(), "{:?}", resumes[0]);
}

#[then(expr = "a toast says {string}")]
fn then_toast(world: &mut TuiWorld, text: String) {
    let notes = drive(world, |h| h.notification_messages());
    assert!(notes.iter().any(|n| n.contains(&text)), "{notes:?}");
}
