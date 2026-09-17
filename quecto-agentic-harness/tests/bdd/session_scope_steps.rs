//! Production-process → socket → typed TUI acceptance for #2009.
use super::*;
use quecto_tui::shell::app::tui_harness::TuiHarness;
use quecto_tui::shell::keys::Key;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct ScopeProcess {
    child: Child,
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    before_history: Option<serde_json::Value>,
    before_state: Option<serde_json::Value>,
    source_home: Option<Vec<u8>>,
}
impl Drop for ScopeProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn command(base: &Path, cwd: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_quecto"));
    command.env("QUECTO_BASE_DIR", base).current_dir(cwd);
    command
}
fn save(base: &Path, cwd: &Path, name: &str, title: &str) {
    let mut child = command(base, cwd)
        .args(["agent", "-s", name, "-m", title])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("save process");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().expect("save status") {
            let output = child.wait_with_output().unwrap();
            assert!(status.success(), "save failed: {output:?}");
            assert!(
                base.join(format!("sessions/cli_{name}.json")).exists(),
                "successful CLI must persist session: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        if Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        } else {
            let _ = child.kill();
            let _ = child.wait();
            panic!("save timed out");
        }
    }
}
#[given("saved production sessions in two different folders")]
fn saved_folders(world: &mut QuectoWorld) {
    let base = world.cli_context.base_dir.as_ref().unwrap();
    let local = base.join("workspace");
    let foreign = base.join("foreign");
    fs::create_dir_all(&foreign).unwrap();
    save(base, &local, "local", "LOCAL-CONVERSATION");
    save(base, &foreign, "foreign", "FOREIGN-CONVERSATION");
}
fn drive<R>(world: &mut QuectoWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    let handle = world.tui_parity_rt.as_ref().unwrap().handle().clone();
    let _guard = handle.enter();
    f(&mut world.tui_parity.as_mut().unwrap().0)
}
fn exchange(world: &mut QuectoWorld) {
    let handle = world.tui_parity_rt.as_ref().unwrap().handle().clone();
    let commands = handle.block_on(world.tui_parity.as_mut().unwrap().0.drain_commands());
    let lists: Vec<_> = commands
        .into_iter()
        .filter(|line| {
            serde_json::from_str::<serde_json::Value>(line).unwrap()["type"] == "list_sessions"
        })
        .collect();
    assert_eq!(lists.len(), 1, "one user action emits one query");
    let request: serde_json::Value = serde_json::from_str(&lists[0]).unwrap();
    let process = world.session_scope_process.as_mut().unwrap();
    writeln!(process.stream, "{}", lists[0]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut line = String::new();
    loop {
        match process.reader.read_line(&mut line) {
            Ok(n) if n > 0 => {
                let value: serde_json::Value = serde_json::from_str(&line).unwrap();
                if value["id"] == request["id"] && value["command"] == "list_sessions" {
                    assert_eq!(value["success"], true, "{value}");
                    world.agent_events.push(line.clone());
                    drive(world, |h| {
                        h.event_line(&line);
                    });
                    return;
                }
                line.clear();
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            result => panic!("socket read failed: {result:?}"),
        }
        assert!(Instant::now() < deadline, "session query timed out");
    }
}
#[when("the operator opens resume through the production socket and TUI")]
fn open_resume(world: &mut QuectoWorld) {
    open_runtime(world, &["-s", "scope-active"]);
}
#[when("the operator opens resume in an ephemeral production runtime")]
fn open_ephemeral(world: &mut QuectoWorld) {
    open_runtime(world, &["--no-session"]);
}
#[when("the operator opens resume with the active local conversation")]
fn open_active(world: &mut QuectoWorld) {
    open_runtime(world, &["-s", "local"]);
}
fn open_runtime(world: &mut QuectoWorld, mode: &[&str]) {
    let base = world.cli_context.base_dir.as_ref().unwrap();
    let socket = base.join("scope.sock");
    let cwd = world.cli_context.cwd.as_ref().expect("execution directory");
    let mut child = command(base, cwd)
        .args(["agent", "--mode", "uds"])
        .args(mode)
        .arg("--socket")
        .arg(&socket)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let stream = loop {
        if let Ok(stream) = UnixStream::connect(&socket) {
            break stream;
        }
        assert!(
            child.try_wait().unwrap().is_none(),
            "runtime exited before socket ready"
        );
        if Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        } else {
            let _ = child.kill();
            let _ = child.wait();
            panic!("socket startup timed out");
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let reader = BufReader::new(stream.try_clone().unwrap());
    world.session_scope_process = Some(ScopeProcess {
        child,
        stream,
        reader,
        before_history: None,
        before_state: None,
        source_home: None,
    });
    let rt = tokio::runtime::Runtime::new().unwrap();
    let h = rt.block_on(TuiHarness::sized(180, 40));
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
    drive(world, |h| {
        h.submit("/resume");
    });
    exchange(world);
}
#[then("only the local saved conversation is displayed")]
fn local_only(world: &mut QuectoWorld) {
    let frame = drive(world, TuiHarness::full_frame);
    assert!(frame.contains("LOCAL-CONVERSATION"), "{frame}");
    assert!(!frame.contains("FOREIGN-CONVERSATION"), "{frame}");
    assert!(
        frame.contains("Local") && frame.contains("Global"),
        "{frame}"
    );
}
#[when("the operator selects Global in the resume picker")]
fn global(world: &mut QuectoWorld) {
    drive(world, |h| {
        h.press(Key::Tab).press(Key::Enter);
    });
    exchange(world);
}
#[then("both saved conversations are displayed with their execution folders")]
fn global_rows(world: &mut QuectoWorld) {
    let frame = drive(world, TuiHarness::full_frame);
    for text in [
        "LOCAL-CONVERSATION",
        "FOREIGN-CONVERSATION",
        "workspace",
        "foreign",
    ] {
        assert!(frame.contains(text), "missing {text}: {frame}");
    }
}

#[when("the operator requests the foreign session by exact key")]
fn exact_foreign(world: &mut QuectoWorld) {
    let base = world.cli_context.base_dir.as_ref().unwrap();
    let path = base.join("sessions/cli_foreign.json");
    world.stdout = fs::read_to_string(&path).expect("source transcript");
    let home = fs::read(base.join("sessions/cli_foreign.home")).unwrap();
    assert_eq!(query(world, "get_session_stats")["sessionKey"], "cli:local");
    let state = query(world, "get_state");
    world.session_scope_process.as_mut().unwrap().before_state = Some(state);
    let history = query(world, "get_messages");
    world.session_scope_process.as_mut().unwrap().before_history = Some(history);
    world.session_scope_process.as_mut().unwrap().source_home = Some(home);
    drive(world, |h| {
        h.press(Key::Escape).submit("/resume cli:foreign");
    });
    let handle = world.tui_parity_rt.as_ref().unwrap().handle().clone();
    let commands = handle.block_on(world.tui_parity.as_mut().unwrap().0.drain_commands());
    let request = commands
        .iter()
        .find(|line| line.contains("\"resume_session\""))
        .expect("exact resume emitted");
    let id = serde_json::from_str::<serde_json::Value>(request).unwrap()["id"].clone();
    let process = world.session_scope_process.as_mut().unwrap();
    writeln!(process.stream, "{request}").unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut line = String::new();
    loop {
        match process.reader.read_line(&mut line) {
            Ok(n) if n > 0 => {
                let value: serde_json::Value = serde_json::from_str(&line).unwrap();
                if value["id"] == id && value["command"] == "resume_session" {
                    world.stderr = line.clone();
                    drive(world, |h| {
                        h.event_line(&line);
                    });
                    return;
                }
                line.clear();
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            result => panic!("socket read failed: {result:?}"),
        }
        assert!(Instant::now() < deadline, "resume response timed out");
    }
}
fn assert_scope_refusal(
    world: &mut QuectoWorld,
    expected: quecto::application::sessions::dto::resume_saved_session::ResumeDisposition,
) {
    use quecto::application::sessions::dto::ResumeSavedSessionError;
    let response: serde_json::Value = serde_json::from_str(&world.stderr).unwrap();
    assert_eq!(response["success"], false, "{response}");
    let error = response["error"]
        .as_str()
        .expect("typed resume error string");
    assert_eq!(
        error,
        ResumeSavedSessionError::Scope(expected).to_string(),
        "{response}"
    );
    assert!(
        error.starts_with("session resume unavailable:"),
        "scope refusal, not ephemeral/generic: {error}"
    );
}

#[then("the runtime refuses replacement as a different-execution-directory scope error")]
fn refused_foreign_directory(world: &mut QuectoWorld) {
    use quecto::application::sessions::dto::ResumeSavedSessionError;
    use quecto::application::sessions::dto::resume_saved_session::ResumeDisposition;
    let expected = ResumeDisposition::DifferentExecutionDirectory;
    assert_scope_refusal(world, expected.clone());
    let error = serde_json::from_str::<serde_json::Value>(&world.stderr).unwrap()["error"]
        .as_str()
        .expect("typed resume error string")
        .to_owned();
    assert_eq!(
        error,
        ResumeSavedSessionError::Scope(expected).to_string(),
        "typed Scope(DifferentExecutionDirectory) refusal"
    );
}

#[then("the runtime refuses replacement as an unavailable-home scope error")]
fn refused_corrupt_home(world: &mut QuectoWorld) {
    use quecto::application::sessions::dto::ResumeSavedSessionError;
    use quecto::application::sessions::dto::resume_saved_session::ResumeDisposition;
    let expected = ResumeDisposition::Unavailable("corrupt authority".into());
    assert_scope_refusal(world, expected.clone());
    let error = serde_json::from_str::<serde_json::Value>(&world.stderr).unwrap()["error"]
        .as_str()
        .expect("typed resume error string")
        .to_owned();
    assert_eq!(
        error,
        ResumeSavedSessionError::Scope(expected).to_string(),
        "typed Scope(Unavailable(_)) refusal"
    );
}

#[then("the selected conversation identity history and ownership are preserved")]
fn preserved_active_identity(world: &mut QuectoWorld) {
    let process = world.session_scope_process.as_ref().unwrap();
    let home = process.source_home.as_ref().unwrap().clone();
    let before = process.before_history.as_ref().unwrap().clone();
    let before_state = process.before_state.as_ref().unwrap().clone();
    let after_state = query(world, "get_state");
    for field in ["sessionKey", "model", "effort", "workflow"] {
        assert_eq!(
            after_state[field], before_state[field],
            "active {field} preserved"
        );
    }
    assert_eq!(before_state["sessionKey"], "cli:local");
    assert!(
        before.to_string().contains("LOCAL-CONVERSATION"),
        "nonempty active history: {before}"
    );
    let after_history = query(world, "get_messages");
    assert_eq!(after_history, before, "active history preserved");
    assert_eq!(query(world, "get_session_stats")["sessionKey"], "cli:local");
    let base = world.cli_context.base_dir.as_ref().unwrap();
    assert_eq!(
        fs::read_to_string(base.join("sessions/cli_foreign.json")).unwrap(),
        world.stdout
    );
    assert_eq!(
        fs::read(base.join("sessions/cli_foreign.home")).unwrap(),
        home
    );
    use quecto::domain::session_identity::SessionIdentity;
    use quecto::infrastructure::persistence::{
        session_layout::FlatSessionLayout, session_ownership::SessionOwnershipGuard,
    };
    let layout = FlatSessionLayout::new(base);
    assert!(
        SessionOwnershipGuard::acquire(&layout, &SessionIdentity::from_persisted_key("cli:local"))
            .is_err(),
        "active owner retained"
    );
    let target = SessionOwnershipGuard::acquire(
        &layout,
        &SessionIdentity::from_persisted_key("cli:foreign"),
    )
    .expect("failed target claim released");
    drop(target);
    let frame = drive(world, TuiHarness::full_frame);
    assert!(
        frame.contains("session resume unavailable"),
        "typed refusal visible: {frame}"
    );
    // ScopeProcess waits for exit; both keys must be claimable after teardown.
    drop(world.session_scope_process.take());
    for key in ["cli:local", "cli:foreign"] {
        let claim =
            SessionOwnershipGuard::acquire(&layout, &SessionIdentity::from_persisted_key(key))
                .expect("runtime teardown releases every owner");
        drop(claim);
    }
}
fn query(world: &mut QuectoWorld, kind: &str) -> serde_json::Value {
    let process = world.session_scope_process.as_mut().unwrap();
    let id = format!(
        "scope-check-{kind}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    writeln!(
        process.stream,
        "{}",
        serde_json::json!({"type": kind, "id": id})
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let mut line = String::new();
        match process.reader.read_line(&mut line) {
            Ok(n) if n > 0 => {
                let value: serde_json::Value = serde_json::from_str(&line).unwrap();
                if value["id"] == id && value["command"] == kind {
                    assert_eq!(value["success"], true, "{value}");
                    return value["data"].clone();
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            result => panic!("socket query failed: {result:?}"),
        }
        assert!(Instant::now() < deadline, "{kind} timed out");
    }
}

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        // A hook-run test inherits GIT_DIR/GIT_WORK_TREE; they must not redirect the fixture.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
#[given("saved production sessions across real Git workspaces")]
fn git_workspaces(world: &mut QuectoWorld) {
    let base = world.cli_context.base_dir.as_ref().unwrap();
    let root = base.join("workspace");
    git(&root, &["init", "-q"]);
    git(
        &root,
        &[
            "-c",
            "user.name=Acceptance",
            "-c",
            "user.email=acceptance@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "initial",
        ],
    );
    let linked = base.join("linked");
    git(
        &root,
        &["worktree", "add", "--detach", linked.to_str().unwrap()],
    );
    let sub = root.join("subfolder");
    let nested = root.join("nested");
    fs::create_dir_all(&sub).unwrap();
    fs::create_dir_all(&nested).unwrap();
    git(&nested, &["init", "-q"]);
    for (folder, name, title) in [
        (&root, "root", "ROOT-CONVERSATION"),
        (&sub, "sub", "SUB-CONVERSATION"),
        (&linked, "linked", "LINKED-CONVERSATION"),
        (&nested, "nested", "NESTED-CONVERSATION"),
    ] {
        save(base, folder, name, title);
    }
}
#[then("the picker groups root subfolder and linked worktree but excludes nested sessions")]
fn git_rows(world: &mut QuectoWorld) {
    let frame = drive(world, TuiHarness::full_frame);
    for title in [
        "ROOT-CONVERSATION",
        "SUB-CONVERSATION",
        "LINKED-CONVERSATION",
    ] {
        assert!(frame.contains(title), "missing {title}: {frame}");
    }
    assert!(!frame.contains("NESTED-CONVERSATION"), "{frame}");
    for path in ["workspace", "subfolder", "linked"] {
        assert!(frame.contains(path), "{frame}");
    }
    let response: serde_json::Value =
        serde_json::from_str(world.agent_events.last().unwrap()).unwrap();
    let rows = response["data"]["sessions"].as_array().unwrap();
    assert_eq!(rows.len(), 3, "exact local identities: {rows:?}");
    // Grouping is not permission: only the row saved in this execution
    // directory is eligible; the subfolder and the linked worktree share the
    // group and are refused.
    for (key, eligible) in [
        ("cli:root", true),
        ("cli:sub", false),
        ("cli:linked", false),
    ] {
        let row = rows
            .iter()
            .find(|row| row["key"] == key)
            .unwrap_or_else(|| panic!("{key} listed: {rows:?}"));
        assert_eq!(row["homeState"], "scoped", "{row}");
        assert_eq!(row["resumeEligible"], eligible, "{row}");
    }
}

#[given("the foreign saved home metadata is corrupt")]
fn corrupt_home(world: &mut QuectoWorld) {
    let base = world.cli_context.base_dir.as_ref().unwrap();
    fs::write(
        base.join("sessions/cli_foreign.home"),
        b"{corrupt authority",
    )
    .unwrap();
}
#[then("both saved conversations remain discoverable without repairing the corrupt home")]
fn corrupt_discovery(world: &mut QuectoWorld) {
    let frame = drive(world, TuiHarness::full_frame);
    for title in ["LOCAL-CONVERSATION", "FOREIGN-CONVERSATION"] {
        assert!(frame.contains(title), "{frame}");
    }
    let base = world.cli_context.base_dir.as_ref().unwrap();
    assert_eq!(
        fs::read(base.join("sessions/cli_foreign.home")).unwrap(),
        b"{corrupt authority"
    );
    let response: serde_json::Value =
        serde_json::from_str(world.agent_events.last().unwrap()).unwrap();
    let row = response["data"]["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["key"] == "cli:foreign")
        .unwrap();
    assert_eq!(row["homeState"], "unavailable");
    assert_eq!(row["resumeEligible"], false);
}
#[then("the ephemeral runtime has published no transcript or home authority")]
fn ephemeral(world: &mut QuectoWorld) {
    let base = world.cli_context.base_dir.as_ref().unwrap();
    let files: Vec<_> = fs::read_dir(base.join("sessions"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert!(
        files
            .iter()
            .all(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("catalogue"))),
        "unexpected durable authority: {files:?}"
    );
    let response: serde_json::Value =
        serde_json::from_str(world.agent_events.last().unwrap()).unwrap();
    assert_eq!(response["data"]["sessions"].as_array().unwrap().len(), 0);
}

#[given("the foreign saved home metadata is absent")]
fn legacy_home(world: &mut QuectoWorld) {
    fs::remove_file(
        world
            .cli_context
            .base_dir
            .as_ref()
            .unwrap()
            .join("sessions/cli_foreign.home"),
    )
    .unwrap();
}
#[then("the foreign conversation is globally visible as unassociated")]
fn legacy_global(world: &mut QuectoWorld) {
    let response: serde_json::Value =
        serde_json::from_str(world.agent_events.last().unwrap()).unwrap();
    let row = response["data"]["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["key"] == "cli:foreign")
        .unwrap();
    assert_eq!(row["homeState"], "legacy_unscoped");
    assert_eq!(row["resumeEligible"], false);
    let frame = drive(world, TuiHarness::full_frame);
    assert!(frame.contains("FOREIGN-CONVERSATION"), "{frame}");
}
#[when("a fresh runtime attempts to start with the foreign session")]
fn startup_foreign(world: &mut QuectoWorld) {
    let base = world.cli_context.base_dir.as_ref().unwrap();
    world.stdout = fs::read_to_string(base.join("sessions/cli_foreign.json")).unwrap();
    let mut child = command(base, &base.join("workspace"))
        .args(["agent", "-s", "foreign", "-m", "must not run"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let output = child.wait_with_output().unwrap();
            assert_eq!(status.code(), Some(1), "startup must refuse: {output:?}");
            world.stderr = String::from_utf8(output.stderr).unwrap();
            return;
        }
        if Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        } else {
            let _ = child.kill();
            let _ = child.wait();
            panic!("startup refusal timed out");
        }
    }
}
#[then("startup refuses without creating home metadata or changing the transcript")]
fn startup_refused(world: &mut QuectoWorld) {
    assert!(!world.stderr.is_empty(), "explicit refusal required");
    let base = world.cli_context.base_dir.as_ref().unwrap();
    assert_eq!(
        fs::read_to_string(base.join("sessions/cli_foreign.json")).unwrap(),
        world.stdout
    );
    assert!(!base.join("sessions/cli_foreign.home").exists());
}
#[given("the derived home catalogue is corrupt")]
fn corrupt_catalogue(world: &mut QuectoWorld) {
    fs::write(
        world
            .cli_context
            .base_dir
            .as_ref()
            .unwrap()
            .join("sessions/home.catalogue"),
        b"corrupt derived cache",
    )
    .unwrap();
}
#[then("the discovery response reports catalogue recovery")]
fn catalogue_rebuilt(world: &mut QuectoWorld) {
    let response: serde_json::Value =
        serde_json::from_str(world.agent_events.last().unwrap()).unwrap();
    assert_eq!(response["data"]["rebuilt"], true);
    assert!(
        response["data"]["diagnostics"]
            .as_array()
            .is_some_and(|d| !d.is_empty()),
        "{response}"
    );
    let base = world.cli_context.base_dir.as_ref().unwrap();
    let catalogue: serde_json::Value =
        serde_json::from_slice(&fs::read(base.join("sessions/home.catalogue")).unwrap()).unwrap();
    assert_eq!(catalogue["version"], 2);
}

#[when("the operator clicks Global in the resume picker")]
fn mouse_global(world: &mut QuectoWorld) {
    let frame = drive(world, TuiHarness::full_frame);
    let (y, line) = frame
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains("[Local]  Global"))
        .unwrap_or_else(|| panic!("visible scope control in:\n{frame}"));
    let byte = line.find("Global").unwrap();
    let x = line[..byte].chars().count();
    drive(world, |h| {
        h.press(Key::MousePress(x as u16, y as u16));
    });
    exchange(world);
}
#[when("the operator cancels the resume picker")]
fn cancel_picker(world: &mut QuectoWorld) {
    drive(world, |h| {
        h.press(Key::Escape);
    });
}
#[then("no history replacement command is sent")]
fn no_resume(world: &mut QuectoWorld) {
    let handle = world.tui_parity_rt.as_ref().unwrap().handle().clone();
    let commands = handle.block_on(world.tui_parity.as_mut().unwrap().0.drain_commands());
    assert!(commands.is_empty(), "Cancel emits no command: {commands:?}");
    let frame = drive(world, TuiHarness::full_frame);
    assert!(!frame.contains("Resume session"), "{frame}");
    assert!(!frame.contains("Saved reply"), "{frame}");
}

#[given("the execution directory is the nested repository")]
fn nested_cwd(world: &mut QuectoWorld) {
    world.cli_context.cwd = Some(
        world
            .cli_context
            .base_dir
            .as_ref()
            .unwrap()
            .join("workspace/nested"),
    );
}
#[then("exactly the nested conversation is discoverable locally")]
fn nested_only(world: &mut QuectoWorld) {
    let response: serde_json::Value =
        serde_json::from_str(world.agent_events.last().unwrap()).unwrap();
    let rows = response["data"]["sessions"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{response}");
    assert_eq!(rows[0]["key"], "cli:nested");
    assert_eq!(rows[0]["resumeEligible"], true);
    let frame = drive(world, TuiHarness::full_frame);
    assert!(frame.contains("NESTED-CONVERSATION"), "{frame}");
}
#[given("the execution directory is a symlink to the local folder")]
fn symlink_cwd(world: &mut QuectoWorld) {
    let base = world.cli_context.base_dir.as_ref().unwrap();
    let alias = base.join("alias");
    std::os::unix::fs::symlink(base.join("workspace"), &alias).unwrap();
    world.cli_context.cwd = Some(alias);
}
#[then("exactly one eligible local identity is returned")]
fn one_local(world: &mut QuectoWorld) {
    let response: serde_json::Value =
        serde_json::from_str(world.agent_events.last().unwrap()).unwrap();
    let rows = response["data"]["sessions"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{response}");
    assert_eq!(rows[0]["key"], "cli:local");
    assert_eq!(rows[0]["resumeEligible"], true);
}

/// Existing in-process fixtures describe a saved session in this runtime's
/// execution directory. Record authority before creating its transcript; legacy
/// scenarios explicitly omit/remove authority instead of bypassing admission.
pub(super) fn record_fixture_home(base: &Path, key: &str) {
    use quecto::application::sessions::ports::session_home::WorkspaceDiscovery;
    use quecto::domain::session_identity::SessionIdentity;
    use quecto::infrastructure::persistence::{
        session_layout::FlatSessionLayout, session_store::FileSessionStore,
    };
    use quecto::infrastructure::workspace::git_scope_discovery::GitScopeDiscovery;
    let identity = SessionIdentity::from_persisted_key(key);
    let store = FileSessionStore::new(FlatSessionLayout::new(base));
    let home = GitScopeDiscovery::default()
        .discover(&std::env::current_dir().unwrap())
        .unwrap();
    store.record_new_home(&identity, &home).unwrap();
    store.release(&identity);
}
