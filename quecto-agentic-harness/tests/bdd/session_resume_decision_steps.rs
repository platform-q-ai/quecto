//! Production-process → socket → typed TUI acceptance for a refused resume
//! (#2011, #2045). The runtime, the store, the folders and the claims are
//! real; the TUI is the production app driven through its harness.
use super::session_scope_steps::{
    assert_scope_refusal, drive, emitted_resume_request, query, resume_roundtrip,
};
use super::*;
use quecto::domain::session_identity::SessionIdentity;
use quecto::infrastructure::persistence::{
    session_layout::FlatSessionLayout, session_ownership::SessionOwnershipGuard,
};
use quecto_tui::shell::app::tui_harness::TuiHarness;
use quecto_tui::shell::keys::Key;
use std::os::unix::fs::PermissionsExt;

/// What only the open notice of a refused resume says, whatever its kind.
const NOTICE_OPEN: &str = "Enter or Esc to close";
fn base(world: &QuectoWorld) -> PathBuf {
    world.cli_context.base_dir.clone().expect("base dir")
}

fn answer(world: &QuectoWorld) -> serde_json::Value {
    serde_json::from_str(&world.stderr).expect("resume answer")
}

/// Everything a refusal must leave alone, observed before the request: the
/// active state and history over the socket, and every session file's bytes.
fn snapshot(world: &mut QuectoWorld) {
    let state = query(world, "get_state");
    let history = query(world, "get_messages");
    let files = session_files(&base(world));
    let process = world.session_scope_process.as_mut().unwrap();
    process.before_state = Some(state);
    process.before_history = Some(history);
    process.store_files = Some(files);
}

/// Name and content of every transcript and home authority in the store.
pub(super) fn session_files(base: &Path) -> String {
    let mut files: Vec<_> = fs::read_dir(base.join("sessions"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("json" | "home")
            )
        })
        .map(|path| {
            format!(
                "{}={}",
                path.display(),
                String::from_utf8_lossy(&fs::read(&path).unwrap())
            )
        })
        .collect();
    files.sort();
    files.join("\n")
}

#[when(expr = "the operator requests the session {string} by exact key")]
fn request_exact(world: &mut QuectoWorld, key: String) {
    snapshot(world);
    drive(world, |h| {
        h.press(Key::Escape).submit(&format!("/resume {key}"));
    });
    let request = emitted_resume_request(world);
    resume_roundtrip(world, &request);
    restore_foreign_access(world);
}

#[when(expr = "a socket client requests the foreign session with action {string}")]
fn request_action(world: &mut QuectoWorld, action: String) {
    snapshot(world);
    let request = serde_json::json!({
        "type": "resume_session",
        "id": format!("s2011-direct-{action}"),
        "session": "cli:foreign",
        "action": action,
    });
    resume_roundtrip(world, &request.to_string());
}

fn listed_row(world: &QuectoWorld, key: &str) -> serde_json::Value {
    let listing: serde_json::Value = world
        .agent_events
        .iter()
        .rev()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .find(|event| event["command"] == "list_sessions")
        .expect("listing");
    let rows = listing["data"]["sessions"].as_array().unwrap();
    let row = rows.iter().find(|row| row["key"] == key);
    row.unwrap_or_else(|| panic!("{key} listed: {listing}"))
        .clone()
}

#[when(expr = "a socket client requests the session {string} with the listed version of {string}")]
fn request_with_another_rows_version(world: &mut QuectoWorld, key: String, other: String) {
    let version = listed_row(world, &other)["homeVersion"].clone();
    assert_ne!(version, listed_row(world, &key)["homeVersion"]);
    snapshot(world);
    let request = serde_json::json!({
        "type": "resume_session", "id": "s2011-wrong-row", "session": key,
        "expectedHomeVersion": version,
    });
    resume_roundtrip(world, &request.to_string());
}

#[then(expr = "the runtime refuses the resume as {string} with code {string}")]
fn refused_as(world: &mut QuectoWorld, kind: String, code: String) {
    let refusal = assert_scope_refusal(world, &kind);
    assert_eq!(refusal["code"], code, "{refusal}");
    assert_eq!(refusal["sessionKey"], "cli:foreign", "{refusal}");
    assert!(
        refusal.get("actions").is_none(),
        "nothing is offered: {refusal}"
    );
}

/// The folder the foreign session was saved in, as the runtime reports it.
fn foreign_folder(world: &QuectoWorld) -> String {
    answer(world)["data"]["executionPath"]
        .as_str()
        .expect("executionPath")
        .to_string()
}

#[then(
    "the refusal carries the command that opens quecto in the foreign folder and the resume step"
)]
fn carries_command(world: &mut QuectoWorld) {
    let data = answer(world)["data"].clone();
    let folder = foreign_folder(world);
    assert_eq!(
        data["command"],
        format!("cd '{folder}' && quecto-tui"),
        "quecto-tui takes no session flag: {data}"
    );
    assert_eq!(data["resume"], "/resume cli:foreign", "{data}");
    assert!(
        std::path::Path::new(&folder).is_dir(),
        "{folder} is the real folder"
    );
}

#[then("the refusal carries no command")]
fn carries_no_command(world: &mut QuectoWorld) {
    let data = answer(world)["data"].clone();
    assert!(data["command"].is_null(), "{data}");
    assert!(
        data["resume"].is_null(),
        "only a session that lives elsewhere is told to go there: {data}"
    );
}

/// The runtime's REAL answer, asked for and rendered by a fresh production
/// TUI on a `columns`×`rows` terminal (the shared one is 180 wide for the
/// picker's rows; a dialog must not depend on that).
fn real_answer_rendered_at(world: &QuectoWorld, columns: usize, rows: usize) -> String {
    let mut answer = answer(world);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut h = rt.block_on(TuiHarness::sized(columns, rows));
    let _guard = rt.enter();
    h.submit(&format!(
        "/resume {}",
        answer["data"]["sessionKey"].as_str().unwrap()
    ));
    let asked = rt.block_on(h.drain_commands());
    let asked = asked
        .iter()
        .find(|line| line.contains("\"resume_session\""));
    let asked: serde_json::Value = serde_json::from_str(asked.expect("asked")).unwrap();
    answer["id"] = asked["id"].clone();
    h.event_line(&answer.to_string());
    h.full_frame()
}

#[then(expr = "the TUI shows the notice titled {string}")]
fn notice_titled(world: &mut QuectoWorld, title: String) {
    let frame = drive(world, TuiHarness::full_frame);
    assert!(frame.contains(&title), "{frame}");
    assert!(frame.contains(NOTICE_OPEN), "{frame}");
    for gone in [
        "Open original folder",
        "Copy into this folder",
        "unavailable",
    ] {
        assert!(
            !frame.contains(gone),
            "nothing is offered ({gone}): {frame}"
        );
    }
    // On an ordinary and on a small terminal the same real answer is whole:
    // the title, the command character for character (wrapped at the column,
    // never elided), the resume step, and the footer inside the border.
    let command = answer(world)["data"]["command"]
        .as_str()
        .expect("command")
        .to_string();
    for (columns, rows) in [(80, 24), (40, 20)] {
        let frame = real_answer_rendered_at(world, columns, rows);
        let inside = |line: &str| {
            let end = line.rfind('│')?;
            let start = line[..end].rfind('│')? + '│'.len_utf8();
            Some(line[start..end].trim().to_string())
        };
        let said: Vec<String> = frame.lines().filter_map(inside).collect();
        let joined = said.join("");
        assert!(
            joined.contains(&command.replace(' ', "")) || said.join(" ").contains(&command),
            "{columns}x{rows}: {command:?} whole: {frame}"
        );
        assert!(
            said.join(" ").contains("/resume cli:foreign"),
            "{columns}x{rows}: {frame}"
        );
        let bottom = frame.lines().position(|l| l.contains('└'));
        let footer = frame.lines().position(|l| l.contains(NOTICE_OPEN));
        assert!(
            footer.is_some() && footer < bottom,
            "{columns}x{rows}: {frame}"
        );
    }
}

#[then(expr = "the runtime refuses the resume with code {string}")]
fn refused_with_code(world: &mut QuectoWorld, code: String) {
    let response = answer(world);
    assert_eq!(response["success"], false, "{response}");
    assert_eq!(response["data"]["outcome"], "refused", "{response}");
    assert_eq!(response["data"]["code"], code, "{response}");
    if code == "action_unavailable" {
        let request = format!(
            "s2011-direct-{}",
            response["data"]["action"].as_str().unwrap()
        );
        assert_eq!(
            response["id"], request,
            "the refused action is the one asked for"
        );
        assert!(
            response["data"]["reason"]
                .as_str()
                .is_some_and(|r| !r.is_empty()),
            "{response}"
        );
    }
}

#[then("the active conversation and every claim are unchanged")]
fn active_unchanged(world: &mut QuectoWorld) {
    let process = world.session_scope_process.as_ref().unwrap();
    let before_state = process.before_state.clone().expect("snapshot");
    let before_history = process.before_history.clone().expect("snapshot");
    let before_files = process.store_files.clone().expect("snapshot");
    let after_state = query(world, "get_state");
    for field in ["sessionKey", "model", "effort", "workflow"] {
        assert_eq!(after_state[field], before_state[field], "active {field}");
    }
    assert_eq!(
        query(world, "get_messages"),
        before_history,
        "active history"
    );
    let base = base(world);
    assert_eq!(
        session_files(&base),
        before_files,
        "no transcript or home changed"
    );
    let layout = FlatSessionLayout::new(&base);
    let active = before_state["sessionKey"].as_str().expect("active key");
    assert!(
        SessionOwnershipGuard::acquire(&layout, &SessionIdentity::from_persisted_key(active))
            .is_err(),
        "the active claim is retained"
    );
    for other in ["cli:foreign", "cli:local", "cli:forei"] {
        if other != active {
            let claim = SessionOwnershipGuard::acquire(
                &layout,
                &SessionIdentity::from_persisted_key(other),
            )
            .unwrap_or_else(|error| panic!("{other} claim leaked: {error}"));
            drop(claim);
        }
    }
}

#[then(expr = "the runtime answers the resume as restored to {string}")]
fn restored_to(world: &mut QuectoWorld, key: String) {
    let response = answer(world);
    assert_eq!(response["success"], true, "{response}");
    assert_eq!(response["data"]["outcome"], "resumed", "{response}");
    assert_eq!(response["data"]["sessionKey"], key, "{response}");
    assert_eq!(query(world, "get_session_stats")["sessionKey"], key);
    let history = query(world, "get_messages").to_string();
    assert!(history.contains("LOCAL-CONVERSATION"), "{history}");
}

#[then("the TUI reports the resumed session")]
fn tui_resumed(world: &mut QuectoWorld) {
    let messages = drive(world, |h| h.notification_messages());
    assert!(
        messages
            .iter()
            .any(|m| m.contains("Resumed session cli:local")),
        "{messages:?}"
    );
    let frame = drive(world, TuiHarness::full_frame);
    assert!(!frame.contains(NOTICE_OPEN), "no decision dialog: {frame}");
}

#[when("the local session home is rewritten after the listing")]
fn rewrite_local_home(world: &mut QuectoWorld) {
    let sessions = base(world).join("sessions");
    let foreign = fs::read(sessions.join("cli_foreign.home")).unwrap();
    assert_ne!(fs::read(sessions.join("cli_local.home")).unwrap(), foreign);
    fs::write(sessions.join("cli_local.home"), foreign).unwrap();
}

#[when("the operator selects the listed local session")]
fn select_listed(world: &mut QuectoWorld) {
    snapshot(world);
    drive(world, |h| {
        h.press(Key::Enter);
    });
    let request = emitted_resume_request(world);
    resume_roundtrip(world, &request);
}

#[then("the resume request carries the listed home version")]
fn carries_listed_version(world: &mut QuectoWorld) {
    let row = listed_row(world, "cli:local");
    let listed = row["homeVersion"].as_str().expect("listed version");
    let process = world.session_scope_process.as_ref().unwrap();
    let request = process.last_resume_request.as_ref().expect("request");
    assert_eq!(request["session"], "cli:local", "{request}");
    assert_eq!(request["expectedHomeVersion"], listed, "{request}");
}

fn running_as_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() == 0 }
}

#[given(
    regex = r"^the foreign session home is (moved away|permission-inaccessible|unreadable metadata)$"
)]
fn foreign_home_state(world: &mut QuectoWorld, state: String) {
    let base = base(world);
    let foreign = base.join("foreign");
    match state.as_str() {
        // Permissions do not bind root: the directory is moved instead, which
        // is the same unobservable home to the discovery adapter.
        "permission-inaccessible" if !running_as_root() => {
            fs::set_permissions(&foreign, fs::Permissions::from_mode(0o000)).unwrap();
            assert!(fs::read_dir(&foreign).is_err(), "access is denied");
        }
        "moved away" | "permission-inaccessible" => {
            fs::rename(&foreign, base.join("foreign-moved")).unwrap();
        }
        _ => fs::write(base.join("sessions/cli_foreign.home"), b"{broken").unwrap(),
    }
}

/// The temp tree must be removable after the scenario.
fn restore_foreign_access(world: &QuectoWorld) {
    let foreign = base(world).join("foreign");
    if foreign.exists() {
        fs::set_permissions(&foreign, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[then("no home metadata was created for the foreign session")]
fn no_foreign_home(world: &mut QuectoWorld) {
    assert!(!base(world).join("sessions/cli_foreign.home").exists());
}

#[when("the operator closes the notice")]
fn cancel_dialog(world: &mut QuectoWorld) {
    let frame = drive(world, TuiHarness::full_frame);
    assert!(frame.contains(NOTICE_OPEN), "dialog open first: {frame}");
    drive(world, |h| {
        h.press(Key::Escape);
    });
}

fn sent_commands(world: &mut QuectoWorld) -> Vec<String> {
    let handle = world.tui_parity_rt.as_ref().unwrap().handle().clone();
    handle.block_on(world.tui_parity.as_mut().unwrap().0.drain_commands())
}

#[then("the notice is closed and no command was sent")]
fn dialog_closed(world: &mut QuectoWorld) {
    let commands = sent_commands(world);
    assert!(
        commands.is_empty(),
        "closing emits no command: {commands:?}"
    );
    let frame = drive(world, TuiHarness::full_frame);
    assert!(!frame.contains(NOTICE_OPEN), "{frame}");
}

#[then("startup refuses naming the other execution directory")]
fn startup_names_directory(world: &mut QuectoWorld) {
    assert!(
        world
            .stderr
            .contains("session 'cli:foreign' cannot start here")
            && world.stderr.contains("different execution directory")
            && world.stderr.contains("\ncd '")
            && world.stderr.contains("then run the same command again")
            && !world.stderr.contains("quecto-tui"),
        "{}",
        world.stderr
    );
}

#[then("the foreign transcript and home metadata are unchanged")]
fn foreign_unchanged(world: &mut QuectoWorld) {
    let sessions = base(world).join("sessions");
    assert_eq!(
        fs::read_to_string(sessions.join("cli_foreign.json")).unwrap(),
        world.stdout
    );
    assert!(sessions.join("cli_foreign.home").exists());
    let layout = FlatSessionLayout::new(base(world));
    let claim = SessionOwnershipGuard::acquire(
        &layout,
        &SessionIdentity::from_persisted_key("cli:foreign"),
    )
    .expect("the refused startup released its claim");
    drop(claim);
}

#[given("the foreign session home names a directory with terminal control characters")]
fn hostile_home(world: &mut QuectoWorld) {
    let hostile = "/tmp/s2011\u{1b}[2J\u{7}evil".as_bytes().to_vec();
    let record = serde_json::json!({
        "version": 1,
        "execution_dir": hostile,
        "group_kind": "folder",
        "group_path": hostile,
        "provenance": "saved_here",
    });
    fs::write(
        base(world).join("sessions/cli_foreign.home"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
}

const INVISIBLE: [char; 5] = ['\u{202e}', '\u{202c}', '\u{200b}', '\u{2066}', '\u{feff}'];

#[given("the foreign session home names a directory with bidi and zero-width characters")]
fn bidi_home(world: &mut QuectoWorld) {
    let hostile: String = format!("/srv/s2011-{}-gpj.exe", String::from_iter(INVISIBLE));
    let hostile = hostile.as_bytes().to_vec();
    let record = serde_json::json!({
        "version": 1,
        "execution_dir": hostile,
        "group_kind": "folder",
        "group_path": hostile,
        "provenance": "saved_here",
    });
    fs::write(
        base(world).join("sessions/cli_foreign.home"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
}

#[then("neither the answer nor the TUI frame carries a bidi or zero-width character")]
fn no_invisible_characters(world: &mut QuectoWorld) {
    let response = answer(world);
    let path = response["data"]["executionPath"].as_str().expect("a path");
    assert!(
        path.contains("gpj.exe"),
        "the recorded folder is shown: {path}"
    );
    let frame = drive(world, TuiHarness::full_frame);
    for text in [response.to_string(), frame] {
        for ch in INVISIBLE {
            assert!(!text.contains(ch), "U+{:04X} in {text:?}", ch as u32);
        }
    }
}

#[then("neither the answer nor the TUI frame carries a raw control character")]
fn no_control_characters(world: &mut QuectoWorld) {
    let response = answer(world);
    let texts = [
        response["error"].as_str().unwrap_or_default().to_string(),
        response["data"].to_string(),
    ];
    for text in &texts {
        assert!(
            !text.contains('\u{1b}') && !text.contains('\u{7}'),
            "{text:?}"
        );
    }
    let frame = drive(world, TuiHarness::full_frame);
    assert!(
        !frame.contains("[2J") && !frame.contains('\u{7}'),
        "{frame:?}"
    );
    let title = "This session's folder can't be read";
    assert!(frame.contains(title), "{frame}");
}
