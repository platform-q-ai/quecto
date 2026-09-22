//! Environments outlive sessions (#2024 S4d), round 4 of review #2033:
//! a master that exits before its coordinator leaves a `running` record
//! whose box has exited (the in-container harness runs its parent-loss
//! shutdown) and whose checkout still hosts a coordination store with an
//! unfinished run. The restore relabels it `retained` — never `stopped`
//! — with the run named (M1a); `gc` keeps any stopped or unrecorded
//! directory whose checkout hosts such a run (M1b); only `container
//! kill` ends it. The rig is `container_persistence_steps`'.
use super::*;

use quecto::domain::environment_registry::EnvironmentStatus;
use quecto::infrastructure::tools::swarm_bridge::{HostedStore, SwarmContext};
use std::sync::Arc;

fn state_dir(world: &QuectoWorld) -> PathBuf {
    base_path(world).join("state")
}

fn environment_id_of(world: &QuectoWorld, env_ref: &str) -> String {
    let document: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(base_path(world).join("environments.json")).unwrap(),
    )
    .unwrap();
    document["environments"][env_ref]["environment_id"]
        .as_str()
        .unwrap_or_else(|| panic!("registry should record {env_ref}"))
        .to_string()
}

/// A real coordination store with a running run created by "coordinator"
/// at `checkout` — the members' checkout of a fake-script environment
/// (its workspace: the fake create script clones nothing).
fn create_running_swarm(checkout: &std::path::Path) -> String {
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    let context = SwarmContext {
        checkout: checkout.to_path_buf(),
        member: "coordinator".into(),
        lifecycle: Arc::new(quecto::application::swarm::LifecycleService),
    };
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 600;
    context
        .create_run(
            &serde_json::json!({"goal":"ship", "constraints":[], "criteria":[{"id":"tests","kind":"command","description":"pass"}], "member_limit":3, "deadline":deadline}),
            &quecto::domain::swarm::ProcessIdentity {
                pid: std::process::id(),
                started: quecto::infrastructure::tools::swarm_bridge::process_start(
                    std::process::id(),
                )
                .unwrap(),
            },
            None,
        )
        .unwrap();
    hosted_run_id(checkout)
}

fn hosted_run_id(checkout: &std::path::Path) -> String {
    HostedStore::at(checkout.to_path_buf())
        .hosted_run()
        .expect("the store is readable")
        .expect("a run was created")
        .id
}

fn checkout_of(world: &QuectoWorld, env_ref: &str) -> PathBuf {
    state_dir(world)
        .join(environment_id_of(world, env_ref))
        .join("workspace")
}

// ─── M1a: the restore retains a gone box hosting an unfinished run ──────────

#[given(expr = "the coordinator of {string} has created a swarm run in its checkout")]
fn given_coordinator_created_run(world: &mut QuectoWorld, env_ref: String) {
    let checkout = checkout_of(world, &env_ref);
    create_running_swarm(&checkout);
}

#[then(
    expr = "stderr should carry the retained-at-restore note for {string} naming its unfinished run"
)]
fn then_stderr_names_unfinished_run(world: &mut QuectoWorld, env_ref: String) {
    let run_id = hosted_run_id(&checkout_of(world, &env_ref));
    let expected = format!(
        "note: {env_ref} retained at restore: run {run_id} unfinished (running); container exited; environment retained for inspection, kill_container to remove"
    );
    assert!(
        world.stderr.lines().any(|line| line == expected),
        "stderr should carry {expected:?}:\n{}",
        world.stderr
    );
}

#[then(
    expr = "the container listing entry {string} should be retained because its run is unfinished"
)]
fn then_listing_entry_unfinished(world: &mut QuectoWorld, env_ref: String) {
    let run_id = hosted_run_id(&checkout_of(world, &env_ref));
    let entry = crate::spawn_env_steps::container_listing_entry(world, &env_ref);
    let reason = entry["metadata"]["retained"].as_str().unwrap_or_default();
    assert!(
        reason.starts_with(&format!(
            "run {run_id} unfinished (running); container exited"
        )),
        "{entry}"
    );
    assert_eq!(entry["status"], "retained", "{entry}");
    assert!(entry["last_error"].is_null(), "{entry}");
}

#[then(expr = "the swarm store of {string} should still hold its unfinished run")]
fn then_store_still_unfinished(world: &mut QuectoWorld, env_ref: String) {
    let run = HostedStore::at(checkout_of(world, &env_ref))
        .hosted_run()
        .expect("the store is readable")
        .expect("the run is still there");
    assert!(run.created() && !run.ended(), "{run:?}");
}

// ─── M1b: gc keeps what hosts an unfinished run, whatever the registry says ─

#[given(
    expr = "the durable environment registry also records a stopped environment {string} named {string} whose state dir hosts an unfinished swarm run and whose fake container has exited"
)]
fn given_stopped_record_hosting_run(world: &mut QuectoWorld, env_ref: String, name: String) {
    let root = state_dir(world);
    let id = crate::container_persistence_round3_steps::plant(
        world,
        &env_ref,
        &name,
        EnvironmentStatus::Stopped,
        &root,
        "exited",
    );
    create_running_swarm(&root.join(id).join("workspace"));
}

#[given(expr = "the state dir of {string} hosts an unfinished swarm run")]
fn given_dir_hosts_run(world: &mut QuectoWorld, id: String) {
    let dir = state_dir(world).join(&id);
    create_running_swarm(&dir.join("workspace"));
    // Planting the store touched the directory: age it again so the
    // collector reads it as abandoned, not as a create in flight.
    crate::container_persistence_steps::age_dir(&dir);
}

/// Whether the gc report keeps `id` for its unfinished run `run_id`, and
/// whether it lists it as removable anywhere before the `kept` block.
fn kept_and_removable(world: &QuectoWorld, id: &str, run_id: &str) -> (bool, bool) {
    let kept = world
        .stdout
        .lines()
        .skip_while(|line| !line.starts_with("kept"))
        .any(|line| {
            line.starts_with(&format!("  {id}  "))
                && line.contains(&format!("hosts swarm run {run_id} (running)"))
                && line.contains("before it can be collected")
        });
    let removable = world
        .stdout
        .lines()
        .take_while(|line| !line.starts_with("kept"))
        .any(|line| line.starts_with(&format!("  {id}  ")));
    (kept, removable)
}

#[then(
    expr = "the gc report should keep the environment of {string} because it hosts an unfinished swarm run"
)]
fn then_gc_keeps_env_hosting_run(world: &mut QuectoWorld, env_ref: String) {
    let id = environment_id_of(world, &env_ref);
    let run_id = hosted_run_id(&checkout_of(world, &env_ref));
    let (kept, removable) = kept_and_removable(world, &id, &run_id);
    assert!(
        kept,
        "gc report should keep {id} for its unfinished run {run_id}:\n{}",
        world.stdout
    );
    assert!(
        !removable,
        "{id} must not be listed as removable:\n{}",
        world.stdout
    );
}

#[then(expr = "the gc report should keep {string} because it hosts an unfinished swarm run")]
fn then_gc_keeps_dir_hosting_run(world: &mut QuectoWorld, id: String) {
    let run_id = hosted_run_id(&state_dir(world).join(&id).join("workspace"));
    let (kept, removable) = kept_and_removable(world, &id, &run_id);
    assert!(
        kept,
        "gc report should keep {id} for its unfinished run {run_id}:\n{}",
        world.stdout
    );
    assert!(
        !removable,
        "{id} must not be listed as removable:\n{}",
        world.stdout
    );
}

// ─── Abandoned runs (#2070) ──────────────────────────────────────────────────

#[then(expr = "the gc report should keep {string} as younger than the {string} abandoned-after")]
fn then_gc_keeps_dir_as_too_young(world: &mut QuectoWorld, id: String, spelled: String) {
    let run_id = hosted_run_id(&state_dir(world).join(&id).join("workspace"));
    let kept = world
        .stdout
        .lines()
        .skip_while(|line| !line.starts_with("kept"))
        .any(|line| {
            line.starts_with(&format!("  {id}  "))
                && line.contains(&format!("hosts swarm run {run_id} (running)"))
                && line.contains(&format!("younger than the {spelled} --abandoned-after"))
        });
    assert!(kept, "{id} should be kept as too young:\n{}", world.stdout);
}

#[then(expr = "the gc report should list {string} as an abandoned swarm run")]
fn then_gc_lists_dir_as_abandoned(world: &mut QuectoWorld, id: String) {
    let run_id = hosted_run_id(&state_dir(world).join(&id).join("workspace"));
    let listed = world
        .stdout
        .lines()
        .take_while(|line| !line.starts_with("kept"))
        .any(|line| {
            line.starts_with(&format!("  {id}  "))
                && line.contains("no registry record")
                && line.contains(&format!("abandoned swarm run {run_id} (running)"))
                && line.contains("collected on --abandoned")
        });
    assert!(
        listed,
        "{id} should be removable as abandoned:\n{}",
        world.stdout
    );
}
