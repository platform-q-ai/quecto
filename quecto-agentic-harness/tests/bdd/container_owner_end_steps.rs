//! #2206: steps for a container child's environment its owner ended for
//! good — its record is forgotten, so the listing no longer names it.

use cucumber::then;

use super::*;
use crate::spawn_env_steps::run_container_command;

#[then(expr = "the container listing should not include {string}")]
fn then_listing_excludes(world: &mut QuectoWorld, env_ref: String) {
    let result = run_container_command(
        world,
        serde_json::json!({"agent_id": "*", "command": "get_containers", "all": true}),
    );
    assert!(
        !result.is_error,
        "get_containers failed: {}",
        result.content
    );
    let parsed: serde_json::Value =
        serde_json::from_str(&result.content).expect("get_containers returns JSON");
    let containers = parsed["containers"].as_array().cloned().unwrap_or_default();
    assert!(
        containers
            .iter()
            .all(|c| c["ref"].as_str() != Some(env_ref.as_str())),
        "{env_ref} should be forgotten: {containers:?}"
    );
}

#[then(expr = "the durable environment registry should not record {string}")]
fn then_registry_forgot(world: &mut QuectoWorld, env_ref: String) {
    let document = crate::container_persistence_steps::registry_document(world);
    assert!(
        document["environments"].get(&env_ref).is_none(),
        "{env_ref} should be forgotten: {document}"
    );
}

#[then("the state dir should hold no environment")]
fn then_state_dir_empty(world: &mut QuectoWorld) {
    let state = base_path(world).join("state");
    let left: Vec<String> = std::fs::read_dir(&state)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    assert!(left.is_empty(), "left in {}: {left:?}", state.display());
}
