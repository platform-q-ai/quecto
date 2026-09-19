//! Production-process → socket → typed TUI acceptance for #2010: the resume
//! picker's search box, answered by the real harness over its real socket.
use super::session_scope_steps::{
    command, drive, emitted_resume_request, git, query, resume_roundtrip, save, socket_roundtrip,
};
use super::*;
use quecto_tui::shell::app::tui_harness::TuiHarness;
use quecto_tui::shell::keys::Key;
use std::os::unix::ffi::OsStrExt;

const TITLES: [&str; 6] = [
    "LOCAL-CONVERSATION otter",
    "ZEBRA-TITLE",
    "KEYED-CONVERSATION",
    "REPO-CONVERSATION",
    "PATH-CONVERSATION",
    "LEGACY otter",
];

#[given("saved production sessions with distinct title key repository and path")]
fn seeded(world: &mut QuectoWorld) {
    let base = world.cli_context.base_dir.as_ref().unwrap();
    let repo = base.join("walrus-repo");
    fs::create_dir_all(repo.join("app")).unwrap();
    git(&repo, &["init", "-q"]);
    for (folder, name, title) in [
        (base.join("workspace"), "local", TITLES[0]),
        (base.join("plain-a"), "bytitle", TITLES[1]),
        (base.join("plain-b"), "bykey", TITLES[2]),
        (repo.join("app"), "byrepo", TITLES[3]),
        (base.join("deep/heron-dir"), "bypath", TITLES[4]),
        (base.join("plain-c"), "legacy", TITLES[5]),
    ] {
        fs::create_dir_all(&folder).unwrap();
        save(base, &folder, name, title);
    }
    // Saved before folders were tracked: no home authority at all.
    fs::remove_file(base.join("sessions/cli_legacy.home")).unwrap();
}

#[given("a production session saved in a folder whose name is not UTF-8")]
fn seeded_odd(world: &mut QuectoWorld) {
    let base = world.cli_context.base_dir.as_ref().unwrap();
    let odd = base.join(std::ffi::OsStr::from_bytes(b"caf\xe9-folder"));
    fs::create_dir_all(&odd).unwrap();
    save(base, &odd, "odd", "ODD-FOLDER-CONVERSATION");
    save(base, &base.join("workspace"), "local", TITLES[0]);
}

#[given("a corrupt session record sits beside the saved sessions")]
fn rotten_record(world: &mut QuectoWorld) {
    let base = world.cli_context.base_dir.as_ref().unwrap();
    fs::write(
        base.join("sessions/cli_rotten.json"),
        b"{\"key\":\"cli:rotten\",",
    )
    .unwrap();
}

/// Relay every discovery request the TUI has emitted to the production
/// socket and its answer back to the TUI, until the TUI has nothing more to
/// ask (an overtaken search makes it send the latest one).
fn pump(world: &mut QuectoWorld) -> Vec<serde_json::Value> {
    let mut answers = Vec::new();
    for _ in 0..16 {
        let handle = world.tui_parity_rt.as_ref().unwrap().handle().clone();
        let commands = handle.block_on(world.tui_parity.as_mut().unwrap().0.drain_commands());
        if commands.is_empty() {
            return answers;
        }
        for request in commands {
            let kind = serde_json::from_str::<serde_json::Value>(&request).unwrap()["type"].clone();
            assert!(
                kind == "search_session_metadata" || kind == "list_sessions",
                "discovery only: {request}"
            );
            let (line, answer) = socket_roundtrip(world, &request);
            assert_eq!(answer["success"], true, "{answer}");
            world.agent_events.push(line.clone());
            drive(world, |h| {
                h.event_line(&line);
            });
            answers.push(answer);
        }
    }
    panic!("the TUI never stopped asking");
}

fn search_answers(world: &QuectoWorld) -> Vec<serde_json::Value> {
    world
        .agent_events
        .iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter(|event| event["command"] == "search_session_metadata")
        .collect()
}

fn focus_search_box(world: &mut QuectoWorld) {
    // Wherever the focus is, Tab until the search row carries the marker.
    for _ in 0..3 {
        let frame = drive(world, TuiHarness::full_frame);
        if frame.contains("▸ Search:") {
            return;
        }
        drive(world, |h| {
            h.press(Key::Tab);
        });
    }
    panic!("the search box never took the focus");
}

#[when(expr = "the operator searches the resume picker for {string}")]
fn search_for(world: &mut QuectoWorld, text: String) {
    focus_search_box(world);
    drive(world, |h| {
        for _ in 0..64 {
            h.press(Key::Backspace);
        }
        for ch in text.chars() {
            h.press(Key::Char(ch));
        }
    });
    pump(world);
}

fn displayed_titles(world: &mut QuectoWorld) -> Vec<&'static str> {
    let frame = drive(world, TuiHarness::full_frame);
    let shown = |title: &&str| frame.contains(*title);
    let mut titles: Vec<_> = TITLES.iter().copied().filter(shown).collect();
    if frame.contains("ODD-FOLDER-CONVERSATION") {
        titles.push("ODD-FOLDER-CONVERSATION");
    }
    titles
}

#[then(expr = "the searched sessions displayed are exactly {string}")]
fn displayed_exactly(world: &mut QuectoWorld, expected: String) {
    let mut expected: Vec<_> = expected.split(", ").collect();
    let mut shown = displayed_titles(world);
    expected.sort_unstable();
    shown.sort_unstable();
    assert_eq!(shown, expected, "{}", drive(world, TuiHarness::full_frame));
}

#[then("no searched session is displayed")]
fn none_displayed(world: &mut QuectoWorld) {
    assert!(displayed_titles(world).is_empty());
    assert!(drive(world, TuiHarness::full_frame).contains("Resume session"));
}

#[then(expr = "every search was answered by the production search command in scope {string}")]
fn all_in_scope(world: &mut QuectoWorld, scope: String) {
    let answers = search_answers(world);
    assert!(answers.len() >= 4, "{answers:?}");
    for answer in &answers {
        assert_eq!(answer["data"]["scope"], scope.as_str(), "{answer}");
        assert!(
            answer["data"]["generation"].as_u64().is_some_and(|g| g > 0),
            "{answer}"
        );
    }
    let last = &answers.last().unwrap()["data"];
    assert_eq!(
        (&last["totalMatches"], &last["searched"]),
        (&serde_json::json!(1), &serde_json::json!(6))
    );
    assert_eq!(last["sessions"][0]["matched"], serde_json::json!(["path"]));
}

#[then(expr = "the last search was answered in scope {string}")]
fn last_in_scope(world: &mut QuectoWorld, scope: String) {
    let answers = search_answers(world);
    let last = &answers.last().expect("a search answer")["data"];
    assert_eq!(last["scope"], scope.as_str(), "{last}");
    assert_eq!(
        last["searched"], 1,
        "only this folder's session was searched: {last}"
    );
}

#[then(expr = "the searched row {string} is labelled unscoped with its stable key {string}")]
fn unscoped_row(world: &mut QuectoWorld, title: String, key: String) {
    let answers = search_answers(world);
    let rows = answers.last().unwrap()["data"]["sessions"]
        .as_array()
        .unwrap()
        .clone();
    let row = rows
        .iter()
        .find(|row| row["title"] == title.as_str())
        .expect("the row");
    assert_eq!(
        (&row["key"], &row["homeState"], &row["executionPath"]),
        (
            &serde_json::json!(key),
            &serde_json::json!("legacy_unscoped"),
            &serde_json::Value::Null
        )
    );
    // The row under the cursor shows its details: move to it and read them.
    let position = rows
        .iter()
        .position(|row| row["title"] == title.as_str())
        .unwrap();
    drive(world, |h| {
        h.press(Key::Tab);
        for _ in 0..position {
            h.press(Key::Down);
        }
    });
    let frame = drive(world, TuiHarness::full_frame);
    assert!(
        frame.contains("No folder recorded (older session)"),
        "{frame}"
    );
    assert!(frame.contains(&format!("ID {key}")), "{frame}");
}

#[then(expr = "the searched row {string} shows its execution folder and its stable key {string}")]
fn scoped_row(world: &mut QuectoWorld, title: String, key: String) {
    let answers = search_answers(world);
    let rows = answers.last().unwrap()["data"]["sessions"]
        .as_array()
        .unwrap()
        .clone();
    let position = rows
        .iter()
        .position(|row| row["title"] == title.as_str())
        .expect("the row");
    let folder = world
        .cli_context
        .cwd
        .as_ref()
        .unwrap()
        .canonicalize()
        .unwrap();
    assert_eq!(rows[position]["executionPath"], folder.to_str().unwrap());
    assert_eq!(rows[position]["resumeEligible"], true);
    drive(world, |h| {
        for _ in 0..rows.len() {
            h.press(Key::Up);
        }
        for _ in 0..position {
            h.press(Key::Down);
        }
    });
    let frame = drive(world, TuiHarness::full_frame);
    let leaf = folder.file_name().unwrap().to_str().unwrap();
    assert!(
        frame.contains(leaf) && frame.contains(&format!("ID {key}")),
        "{frame}"
    );
}

#[when("the operator switches the resume picker back to Local Folder")]
fn back_to_local(world: &mut QuectoWorld) {
    for _ in 0..3 {
        if drive(world, TuiHarness::full_frame).contains("▸ Scope:") {
            break;
        }
        drive(world, |h| {
            h.press(Key::Tab);
        });
    }
    drive(world, |h| {
        h.press(Key::Left);
    });
    pump(world);
    assert!(drive(world, TuiHarness::full_frame).contains("[Local Folder]"));
}

#[then("the search answer reports every saved session searched and none matched")]
fn none_matched(world: &mut QuectoWorld) {
    let answers = search_answers(world);
    let last = &answers.last().unwrap()["data"];
    assert_eq!(
        (&last["searched"], &last["totalMatches"]),
        (&serde_json::json!(6), &serde_json::json!(0)),
        "{last}"
    );
    let base = world.cli_context.base_dir.as_ref().unwrap();
    let transcript = fs::read_to_string(base.join("sessions/cli_bytitle.json")).unwrap();
    assert!(
        transcript.contains("Saved reply"),
        "the content really is in the transcripts"
    );
}

#[when(expr = "the operator types {string} then {string} before the first answer arrives")]
fn type_past(world: &mut QuectoWorld, first: String, rest: String) {
    focus_search_box(world);
    drive(world, |h| {
        for ch in first.chars().chain(rest.chars()) {
            h.press(Key::Char(ch));
        }
    });
    let handle = world.tui_parity_rt.as_ref().unwrap().handle().clone();
    let commands = handle.block_on(world.tui_parity.as_mut().unwrap().0.drain_commands());
    assert_eq!(commands.len(), 1, "single flight: {commands:?}");
    let request: serde_json::Value = serde_json::from_str(&commands[0]).unwrap();
    assert_eq!(request["query"], first.as_str());
    let (line, answer) = socket_roundtrip(world, &commands[0]);
    assert!(
        answer["data"]["totalMatches"].as_u64().unwrap() >= 1,
        "{answer}"
    );
    world.agent_events.push(line.clone());
    drive(world, |h| {
        h.event_line(&line);
    });
}

#[then("the first answer is shown as progress while the picker keeps searching")]
fn first_is_progress(world: &mut QuectoWorld) {
    // What `z` matched depends on the temp directory's random name, so the
    // rows on screen are compared with the answer itself: exactly its rows,
    // no longer the full listing.
    let answers = search_answers(world);
    let rows = answers.last().unwrap()["data"]["sessions"]
        .as_array()
        .unwrap()
        .clone();
    let mut answered: Vec<_> = rows
        .iter()
        .map(|row| row["title"].as_str().unwrap())
        .collect();
    let mut shown = displayed_titles(world);
    answered.sort_unstable();
    shown.sort_unstable();
    assert_eq!(shown, answered);
    assert!(
        shown.len() < TITLES.len() && shown.contains(&"ZEBRA-TITLE"),
        "{shown:?}"
    );
    let frame = drive(world, TuiHarness::full_frame);
    assert!(frame.contains("Sessions · Searching…"), "{frame}");
}

#[then("the picker has settled")]
fn picker_settled(world: &mut QuectoWorld) {
    let frame = drive(world, TuiHarness::full_frame);
    assert!(!frame.contains("Searching…"), "{frame}");
}

#[then(expr = "the answer to {string} replaces the listing with exactly {string}")]
fn latest_shown(world: &mut QuectoWorld, query: String, title: String) {
    let answers = pump(world);
    assert_eq!(
        answers.len(),
        1,
        "exactly the latest text was asked: {answers:?}"
    );
    assert_eq!(answers[0]["data"]["query"], query.as_str());
    assert_eq!(displayed_titles(world), [title.as_str()]);
}

fn direct_search(world: &mut QuectoWorld, query: &str) -> (String, serde_json::Value) {
    let request = serde_json::json!({
        "type": "search_session_metadata", "id": format!("direct-{}", world.agent_events.len()),
        "query": query, "scope": "global", "generation": 1,
    });
    let (line, answer) = socket_roundtrip(world, &request.to_string());
    world.agent_events.push(line.clone());
    (line, answer)
}

#[then("a production search for each hostile query is answered safely the same way twice")]
fn hostile_queries(world: &mut QuectoWorld) {
    const NOTHING: &[&str] = &[];
    let literal_text_no_metadata_contains = [
        ".*",
        "[a-z]+",
        "(zebra|otter)",
        "^ZEBRA",
        "*",
        "?",
        "**/*",
        "\\",
        "$(touch pwned)",
        "`id`",
        "'; DROP TABLE sessions;--",
        "../../etc/passwd",
        // An escape and a bidi override are dropped; what is left is literal.
        "\u{1b}[2J\u{7}",
        "\u{202e}arbez",
    ];
    let mut cases: Vec<(&str, &[&str])> = literal_text_no_metadata_contains
        .iter()
        .map(|query| (*query, NOTHING))
        .collect();
    // Invisible characters are not searched for: nothing visible names every
    // session, and a hidden character inside a word does not split it.
    cases.push(("\u{200b}", &TITLES));
    cases.push(("\u{0}", &TITLES));
    cases.push(("zebra\u{200b}-title", &TITLES[1..2]));
    for (query, expected) in cases {
        let (_, first) = direct_search(world, query);
        assert_eq!(first["success"], true, "{query:?}: {first}");
        let (_, second) = direct_search(world, query);
        assert_eq!(
            first["data"]["sessions"], second["data"]["sessions"],
            "{query:?}: deterministic"
        );
        let echoed = first["data"]["query"].as_str().expect("the echoed query");
        assert!(
            !echoed
                .chars()
                .any(|ch| ch.is_control() || matches!(ch, '\u{202e}' | '\u{200b}')),
            "{query:?} was echoed raw: {echoed:?}"
        );
        let mut titles: Vec<_> = first["data"]["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["title"].as_str().unwrap().to_string())
            .collect();
        let mut expected: Vec<_> = expected.iter().map(|title| title.to_string()).collect();
        titles.sort();
        expected.sort();
        assert_eq!(titles, expected, "{query:?}");
    }
    let base = world.cli_context.base_dir.as_ref().unwrap();
    let cwd = world.cli_context.cwd.as_ref().unwrap();
    assert!(!base.join("pwned").exists() && !cwd.join("pwned").exists());
}

#[then(expr = "a production search for a query of {int} characters is refused without searching")]
fn long_query(world: &mut QuectoWorld, chars: usize) {
    let (line, answer) = direct_search(world, &"z".repeat(chars));
    assert_eq!(answer["success"], true, "{answer}");
    let data = &answer["data"];
    assert_eq!(
        data["refused"],
        format!("query too long: {chars} characters (at most 256 are searched)")
    );
    assert_eq!(
        (&data["searched"], &data["totalMatches"]),
        (&serde_json::json!(0), &serde_json::json!(0))
    );
    assert!(
        line.len() < 2048,
        "the echo is bounded: {} bytes",
        line.len()
    );
}

#[then(
    "the search answer is valid UTF-8 with the byte that is no text spelled in the execution path"
)]
fn odd_folder(world: &mut QuectoWorld) {
    let answers = search_answers(world);
    let row = &answers.last().unwrap()["data"]["sessions"][0];
    let path = row["executionPath"].as_str().expect("a path");
    assert!(path.ends_with("caf\\xE9-folder"), "{path}");
    assert_eq!(row["matched"], serde_json::json!(["repository", "path"]));
    let frame = drive(world, TuiHarness::full_frame);
    assert!(frame.contains("-folder"), "{frame}");
}

fn snapshot_active(world: &mut QuectoWorld) {
    let state = query(world, "get_state");
    let history = query(world, "get_messages");
    let files = super::session_resume_decision_steps::session_files(
        world.cli_context.base_dir.as_ref().unwrap(),
    );
    let process = world.session_scope_process.as_mut().unwrap();
    process.before_state = Some(state);
    process.before_history = Some(history);
    process.store_files = Some(files);
}

fn select_searched_row(world: &mut QuectoWorld) {
    drive(world, |h| {
        h.press(Key::Tab).press(Key::Enter);
    });
    let request = emitted_resume_request(world);
    let sent: serde_json::Value = serde_json::from_str(&request).unwrap();
    assert_eq!(sent["session"], "cli:bytitle");
    assert!(
        sent["expectedHomeVersion"]
            .as_str()
            .is_some_and(|v| v.starts_with("h1-")),
        "{sent}"
    );
    resume_roundtrip(world, &request);
}

#[when("the searched session is deleted before the operator selects it")]
fn deleted_then_selected(world: &mut QuectoWorld) {
    snapshot_active(world);
    let base = world.cli_context.base_dir.as_ref().unwrap();
    fs::remove_file(base.join("sessions/cli_bytitle.json")).unwrap();
    select_searched_row(world);
}

#[when("the searched session is re-homed before the operator selects it")]
fn rehomed_then_selected(world: &mut QuectoWorld) {
    snapshot_active(world);
    let base = world.cli_context.base_dir.as_ref().unwrap();
    fs::copy(
        base.join("sessions/cli_bykey.home"),
        base.join("sessions/cli_bytitle.home"),
    )
    .unwrap();
    select_searched_row(world);
}

fn assert_active_unchanged(world: &mut QuectoWorld) {
    let process = world.session_scope_process.as_ref().unwrap();
    let (state, history) = (
        process.before_state.clone().unwrap(),
        process.before_history.clone().unwrap(),
    );
    let after = query(world, "get_state");
    assert_eq!(after["sessionKey"], state["sessionKey"]);
    assert_eq!(after["sessionKey"], "cli:local");
    assert_eq!(query(world, "get_messages"), history);
}

#[then("the selection is refused as not found and the active conversation is unchanged")]
fn refused_not_found(world: &mut QuectoWorld) {
    let answer: serde_json::Value = serde_json::from_str(&world.stderr).unwrap();
    assert_eq!(
        (&answer["success"], &answer["data"]["code"]),
        (&serde_json::json!(false), &serde_json::json!("not_found")),
        "{answer}"
    );
    assert_active_unchanged(world);
}

#[then("the selection is refused as a stale home version and the active conversation is unchanged")]
fn refused_stale(world: &mut QuectoWorld) {
    let answer: serde_json::Value = serde_json::from_str(&world.stderr).unwrap();
    assert_eq!(
        (&answer["success"], &answer["data"]["code"]),
        (
            &serde_json::json!(false),
            &serde_json::json!("stale_home_version")
        ),
        "{answer}"
    );
    assert_active_unchanged(world);
    let notes = drive(world, |h| h.notification_messages());
    assert!(
        notes.iter().any(|n| n.contains("List out of date")),
        "{notes:?}"
    );
}

#[then("no resume was requested and no saved session or home changed")]
fn escape_changed_nothing(world: &mut QuectoWorld) {
    let handle = world.tui_parity_rt.as_ref().unwrap().handle().clone();
    let commands = handle.block_on(world.tui_parity.as_mut().unwrap().0.drain_commands());
    assert!(commands.is_empty(), "Escape emits no command: {commands:?}");
    assert!(!drive(world, TuiHarness::full_frame).contains("Resume session"));
    let base = world.cli_context.base_dir.as_ref().unwrap().clone();
    let before = super::session_resume_decision_steps::session_files(&base);
    assert_eq!(query(world, "get_state")["sessionKey"], "cli:local");
    let after = super::session_resume_decision_steps::session_files(&base);
    assert_eq!(before, after);
    let homes = fs::read_dir(base.join("sessions"))
        .unwrap()
        .filter_map(Result::ok);
    let homes = homes
        .filter(|e| e.path().extension().is_some_and(|x| x == "home"))
        .count();
    assert_eq!(
        homes, 5,
        "the unscoped session acquired no home by being searched"
    );
}

#[then("the search answers name the corrupt record and a later search reports no recovery")]
fn corrupt_diagnosed(world: &mut QuectoWorld) {
    let events: Vec<serde_json::Value> = world
        .agent_events
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let listing = events
        .iter()
        .find(|e| e["command"] == "list_sessions")
        .expect("the opening listing");
    assert_eq!(listing["data"]["rebuilt"], true, "{listing}");
    let search = events
        .iter()
        .rev()
        .find(|e| e["command"] == "search_session_metadata")
        .unwrap();
    let diagnostics = search["data"]["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|d| d.as_str().unwrap().starts_with("cli_rotten.json: ")),
        "{diagnostics:?}"
    );
    assert_eq!(
        search["data"]["rebuilt"], false,
        "recovered once, by the listing"
    );
    // Corrupted again, a search recovers it by itself and says so.
    let base = world.cli_context.base_dir.as_ref().unwrap();
    fs::write(base.join("sessions/home.catalogue"), b"\x00garbage").unwrap();
    let (_, answer) = direct_search(world, "zebra-title");
    assert_eq!(answer["data"]["rebuilt"], true, "{answer}");
    assert_eq!(answer["data"]["sessions"].as_array().unwrap().len(), 1);
}

#[then(expr = "exact-key resume of {string} is still answered with a decision")]
fn exact_key_independent(world: &mut QuectoWorld, key: String) {
    let base = world.cli_context.base_dir.as_ref().unwrap();
    fs::write(base.join("sessions/home.catalogue"), b"\x00garbage").unwrap();
    let request = serde_json::json!({"type": "resume_session", "id": "exact-1", "session": key});
    let (_, answer) = socket_roundtrip(world, &request.to_string());
    assert_eq!(answer["data"]["outcome"], "decision", "{answer}");
    assert_eq!(answer["data"]["kind"], "cross_folder", "{answer}");
    let _ = command;
}

fn raw_search(world: &mut QuectoWorld, id: &str, fields: serde_json::Value) -> serde_json::Value {
    let mut request = serde_json::json!({
        "type": "search_session_metadata", "id": id, "query": "otter", "scope": "global",
    });
    for (field, value) in fields.as_object().unwrap() {
        request[field] = value.clone();
    }
    let (line, answer) = socket_roundtrip(world, &request.to_string());
    world.agent_events.push(line);
    assert_eq!(
        (&answer["id"], &answer["success"]),
        (&serde_json::json!(id), &serde_json::json!(true)),
        "{answer}"
    );
    answer["data"].clone()
}

#[then(
    expr = "a production search whose limit is the text {string} is refused under its own id without searching"
)]
fn limit_not_a_number(world: &mut QuectoWorld, limit: String) {
    let data = raw_search(
        world,
        "lenient-1",
        serde_json::json!({"limit": limit, "generation": 4}),
    );
    assert_eq!(data["refused"], "limit must be a number", "{data}");
    assert_eq!(
        (&data["generation"], &data["searched"]),
        (&serde_json::json!(4), &serde_json::json!(0))
    );
    assert!(data["sessions"].as_array().unwrap().is_empty(), "{data}");
}

#[then(
    expr = "a production search with limit -1 and generation 7 is answered with limit {int} and generation {int}"
)]
fn numbers_brought_into_range(world: &mut QuectoWorld, limit: u64, generation: u64) {
    let data = raw_search(
        world,
        "lenient-2",
        serde_json::json!({"limit": -1, "generation": 7}),
    );
    assert!(data["refused"].is_null(), "{data}");
    assert_eq!(
        (&data["limit"], &data["generation"]),
        (&serde_json::json!(limit), &serde_json::json!(generation))
    );
    assert_eq!(
        data["sessions"].as_array().unwrap().len(),
        1,
        "the limit holds: {data}"
    );
    assert_eq!(data["totalMatches"], 2, "{data}");
}

#[then(
    "a production search whose generation is the fraction 7.9 is refused under its own id without searching"
)]
fn generation_not_an_integer(world: &mut QuectoWorld) {
    let data = raw_search(world, "exact-1", serde_json::json!({"generation": 7.9}));
    assert_eq!(
        data["refused"], "generation must be an integer from 0 to 18446744073709551615",
        "{data}"
    );
    assert_eq!(
        (&data["generation"], &data["searched"]),
        (&serde_json::json!(0), &serde_json::json!(0)),
        "never a rounded echo: {data}"
    );
}

#[then(
    expr = "a production search whose limit is the number 1e400 is answered under its own id with limit {int}"
)]
fn limit_no_float_holds(world: &mut QuectoWorld, limit: u64) {
    let request = r#"{"type":"search_session_metadata","id":"big-1","query":"otter","scope":"global","generation":3,"limit":1e400}"#;
    let (_, answer) = socket_roundtrip(world, request);
    let data = &answer["data"];
    assert_eq!(answer["success"], true, "{answer}");
    assert!(data["refused"].is_null(), "{answer}");
    assert_eq!(
        (&data["limit"], &data["generation"]),
        (&serde_json::json!(limit), &serde_json::json!(3))
    );
}
