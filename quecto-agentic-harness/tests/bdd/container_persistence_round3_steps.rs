//! Environments outlive sessions (#2024 S4d), round 3 of review #2033:
//! a `retained` environment (#1924) with an exited container is kept by
//! `restore`, `container gc --dry-run` and `container gc` alike and ended
//! only by `container kill` (H1); a record whose workspace lies outside
//! the config's state dir is reported, never scanned or removed through
//! the config's cleanup (M1). The rig is `container_persistence_steps`'.
use super::*;

use quecto::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentStatus,
};

fn state_dir(world: &QuectoWorld) -> PathBuf {
    base_path(world).join("state")
}

fn runtime_dir(world: &QuectoWorld) -> PathBuf {
    base_path(world).join("fake-runtime")
}

fn registry_path(world: &QuectoWorld) -> PathBuf {
    base_path(world).join("environments.json")
}

fn environment_id_of(world: &QuectoWorld, env_ref: &str) -> String {
    let document: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(registry_path(world)).unwrap()).unwrap();
    document["environments"][env_ref]["environment_id"]
        .as_str()
        .unwrap_or_else(|| panic!("registry should record {env_ref}"))
        .to_string()
}

/// The fake script set's operations logged for one environment id.
fn operations_on(world: &QuectoWorld, id: &str) -> Vec<String> {
    std::fs::read_to_string(base_path(world).join("persist-log.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|entry| entry["env_id"] == id)
        .filter_map(|entry| entry["kind"].as_str().map(str::to_string))
        .collect()
}

/// A record another session left, with the fake script set's argv
/// retained (the persistence rig's scripts) and a state dir laid out the
/// way the scripts lay one out under `root`.
fn plant(
    world: &QuectoWorld,
    env_ref: &str,
    name: &str,
    status: EnvironmentStatus,
    root: &std::path::Path,
    container_state: &str,
) -> String {
    let base = base_path(world);
    let number: u64 = env_ref[1..].parse().unwrap();
    let id = format!("env-planted-{number}");
    let dir = root.join(&id);
    std::fs::create_dir_all(dir.join("workspace")).unwrap();
    std::fs::write(dir.join("container"), format!("quecto-{id}\n")).unwrap();
    std::fs::write(
        runtime_dir(world).join(format!("quecto-{id}")),
        format!("{container_state}\n"),
    )
    .unwrap();
    let script = |name: &str| base.join(name).to_string_lossy().into_owned();
    quecto::composition::environments::build_environment_registry_store(&base)
        .record(&EnvironmentRecord {
            environment_ref: env_ref.to_string(),
            environment_id: id.clone(),
            environment_uuid: format!("uuid-{number}"),
            name: Some(name.to_string()),
            workspace_path: dir.join("workspace"),
            repository: String::new(),
            script_name: "default".into(),
            retained_exec_argv: vec![script("persist-exec.sh")],
            retained_kill_argv: vec![script("persist-kill.sh"), "kill".into()],
            retained_cleanup_argv: vec![script("persist-kill.sh"), "cleanup".into()],
            retained_inspect_argv: vec![script("persist-inspect.sh")],
            members: vec![],
            status,
            metadata: serde_json::json!({"retained": "swarm run ended"}),
            last_error: None,
            origin: EnvironmentOrigin::Created,
            created_by: "elsewhere".into(),
            created_at: Some(0),
        })
        .unwrap();
    id
}

// ─── H1: retained survives everything but an explicit kill ─────────────────

#[given(
    expr = "the durable environment registry also records a retained environment {string} named {string} whose fake container has exited"
)]
fn given_retained_record(world: &mut QuectoWorld, env_ref: String, name: String) {
    let root = state_dir(world);
    plant(
        world,
        &env_ref,
        &name,
        EnvironmentStatus::Retained,
        &root,
        "exited",
    );
}

#[given("the durable environment registry on disk is noted")]
fn given_registry_noted(world: &mut QuectoWorld) {
    world.registry_snapshot = Some(std::fs::read(registry_path(world)).unwrap());
}

#[then("the durable environment registry on disk should be byte-identical to the noted one")]
fn then_registry_unchanged(world: &mut QuectoWorld) {
    let noted = world
        .registry_snapshot
        .as_ref()
        .expect("a step noted the registry first");
    let now = std::fs::read(registry_path(world)).unwrap();
    assert!(
        &now == noted,
        "the registry document changed:\nbefore: {}\nafter: {}",
        String::from_utf8_lossy(noted),
        String::from_utf8_lossy(&now)
    );
}

#[then(
    expr = "the gc report should keep the environment of {string} as retained until an explicit kill"
)]
fn then_gc_keeps_retained(world: &mut QuectoWorld, env_ref: String) {
    let id = environment_id_of(world, &env_ref);
    let kept = world
        .stdout
        .lines()
        .skip_while(|line| !line.starts_with("kept"))
        .any(|line| {
            line.starts_with(&format!("  {id}  "))
                && line.contains(&format!("recorded {env_ref} as retained"))
                && line.contains(&format!("quecto container kill {env_ref}"))
        });
    assert!(
        kept,
        "gc report should keep {id} as retained:\n{}",
        world.stdout
    );
    assert!(
        !world.stdout.contains(&format!("  {id}  ")) || kept,
        "{}",
        world.stdout
    );
}

#[then(expr = "the persistent runtime should never have removed the environment of {string}")]
fn then_never_removed(world: &mut QuectoWorld, env_ref: String) {
    let id = environment_id_of(world, &env_ref);
    let operations = operations_on(world, &id);
    assert!(
        !operations.iter().any(|op| op == "kill" || op == "cleanup"),
        "no kill or cleanup should have run for {id}: {operations:?}"
    );
}
