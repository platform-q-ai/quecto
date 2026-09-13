//! BDD steps for the real-process fleet teardown entry points (#1938):
//! delete_all_subagents, the termination signal, the last client of the
//! default lifetime, a persistent harness, and a script-managed member's
//! finalization on a session switch — all against the in-process restoring
//! harness and real launched child of `restore_lifetime_steps`.
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use quecto::domain::session::SessionStore;
use quecto::infrastructure::persistence::session_store::FileSessionStore;
use quecto::infrastructure::tools::subagent_registry::SubagentEntry;

use crate::QuectoWorld;
use crate::restore_lifetime_steps::{base, process_alive, read_until, send, state};

// ── Fleet teardown entry points (#1938) ──────────────────────────────────────

#[when("the client sends delete_all_subagents to the restoring harness")]
fn when_delete_all(world: &mut QuectoWorld) {
    let ack = send(
        world,
        "{\"type\":\"delete_all_subagents\",\"id\":\"del-1\"}",
        "del-1",
    );
    state(world).harness.as_mut().unwrap().resume_ack = Some(ack);
}

#[then(expr = "the delete response reports {int} removed with every child settled")]
fn then_delete_settled(world: &mut QuectoWorld, removed: u64) {
    let ack = state(world)
        .harness
        .as_ref()
        .unwrap()
        .resume_ack
        .clone()
        .expect("delete ack");
    assert_eq!(ack["success"], true, "{ack}");
    assert_eq!(ack["data"]["removed"], removed, "{ack}");
    assert_eq!(
        ack["data"]["unsettled"].as_array().map(Vec::len),
        Some(0),
        "{ack}"
    );
    for settled in ack["data"]["settled"].as_array().unwrap() {
        assert_eq!(settled["result"], "graceful", "{settled}");
    }
}

#[when("a termination signal is delivered to the restoring harness")]
fn when_termination_signal(world: &mut QuectoWorld) {
    assert!(state(world).harness.is_some());
    quecto::interface::cli::deliver_termination_signal();
}

#[then(expr = "the restoring harness is still serving after {int} seconds")]
fn then_still_serving(world: &mut QuectoWorld, seconds: u64) {
    std::thread::sleep(Duration::from_secs(seconds));
    let harness = state(world).harness.as_mut().unwrap();
    assert!(
        !harness.handle.as_ref().unwrap().is_finished(),
        "a persistent harness must survive its last client"
    );
    assert!(harness.socket_path.exists());
    // A fresh client is served.
    let mut client = UnixStream::connect(&harness.socket_path).expect("reconnect");
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    client
        .write_all(b"{\"type\":\"get_state\",\"id\":\"alive-1\"}\n")
        .unwrap();
    let reply = read_until(&mut client, |v| v["id"] == "alive-1").expect("served");
    assert_eq!(reply["success"], true, "{reply}");
    harness.client = Some(client);
}

#[then("no saved session writes a live child")]
fn then_no_session_writes_a_live_child(world: &mut QuectoWorld) {
    let base = base(world);
    let store = FileSessionStore::new(&base);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let sessions = rt.block_on(store.list(None)).unwrap();
    assert!(
        !sessions.is_empty(),
        "the session was persisted on the way out"
    );
    for summary in sessions {
        let saved = rt
            .block_on(store.load(&summary.key))
            .unwrap()
            .expect("loadable");
        assert!(
            saved
                .subagent_roster
                .iter()
                .all(|row| row.liveness != quecto::domain::session::SubagentLiveness::Live),
            "{}: no live operational child may be persisted: {:?}",
            summary.key,
            saved.subagent_roster
        );
    }
}

#[given("a script-managed member with a fake kill argv is registered in the restoring harness")]
fn given_script_member(world: &mut QuectoWorld) {
    let base = base(world);
    let log = base.join("member-kill.log");
    let script = base.join("member-kill.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf 'killed %s\\n' \"$QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'\n",
            log.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let harness = state(world).harness.as_ref().unwrap();
    let uuid = quecto::domain::ids::AgentUuid::mint();
    let mut entry = SubagentEntry::with_identity(
        uuid.clone(),
        "container-member".into(),
        base.join("container-member-never.sock"),
        0,
    );
    entry.launch_generation = Some(quecto::domain::subagent_teardown::LaunchGeneration::new(
        7_001,
    ));
    entry.cleanup_environment_id = Some("env-member".into());
    entry.cleanup_argv = vec![script.display().to_string()];
    harness
        .registry
        .lock()
        .unwrap()
        .insert(uuid.into_string(), entry);
}

#[then("the fake kill argv of the script-managed member ran once")]
fn then_script_member_killed(world: &mut QuectoWorld) {
    let log = base(world).join("member-kill.log");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !log.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let text = std::fs::read_to_string(&log).expect("the member's kill argv ran");
    assert_eq!(text.lines().collect::<Vec<_>>(), vec!["killed env-member"]);
}

#[then(expr = "the child process of the re-spawned {string} is gone within {int} seconds")]
fn then_respawned_gone(world: &mut QuectoWorld, agent_id: String, seconds: u64) {
    let entry = state(world)
        .respawned
        .clone()
        .expect("re-spawned entry captured");
    assert_eq!(entry.display_name, agent_id);
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while process_alive(entry.pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!process_alive(entry.pid), "the child must be gone");
    let supervisor = entry.owned_child_supervisor.clone().expect("supervisor");
    let handle = entry.owned_child.expect("owned handle");
    assert!(
        supervisor.signals_sent(handle).is_empty(),
        "ended by protocol, not a signal: {:?}",
        supervisor.signals_sent(handle)
    );
}
