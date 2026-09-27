//! #2220: steps for the bounded `agent_cmd get_containers` listing — what
//! the model is handed, read from the last container command's result.

use cucumber::{then, when};

use super::*;
use crate::spawn_env_steps::{container_listing_entry, run_container_command};

#[when("I run container command \"get_containers\" with all")]
fn when_get_containers_all(world: &mut QuectoWorld) {
    let result = run_container_command(
        world,
        serde_json::json!({"agent_id": "*", "command": "get_containers", "all": true}),
    );
    world.container_cmd_result = Some(result);
}

fn last_listing(world: &QuectoWorld) -> serde_json::Value {
    let result = world
        .container_cmd_result
        .as_ref()
        .expect("no container command result");
    assert!(
        !result.is_error,
        "get_containers failed: {}",
        result.content
    );
    serde_json::from_str(&result.content)
        .unwrap_or_else(|e| panic!("get_containers returns JSON: {e}; got {}", result.content))
}

fn row<'a>(listing: &'a serde_json::Value, env_ref: &str) -> &'a serde_json::Value {
    listing["containers"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["ref"] == env_ref))
        .unwrap_or_else(|| panic!("the listing should show {env_ref}: {listing}"))
}

#[then(expr = "the listing should show refs {string} in that order out of {int}")]
fn then_listing_refs(world: &mut QuectoWorld, refs: String, total: u64) {
    let listing = last_listing(world);
    let shown: Vec<&str> = listing["containers"]
        .as_array()
        .unwrap_or_else(|| panic!("containers: {listing}"))
        .iter()
        .map(|row| row["ref"].as_str().unwrap_or_default())
        .collect();
    let expected: Vec<&str> = refs.split(", ").collect();
    assert_eq!(shown, expected, "{listing}");
    assert_eq!(listing["total"].as_u64(), Some(total), "{listing}");
}

#[then(expr = "the listing should count {int} hidden, naming \"all\":true")]
fn then_listing_hidden(world: &mut QuectoWorld, hidden: u64) {
    let listing = last_listing(world);
    assert_eq!(listing["hidden"].as_u64(), Some(hidden), "{listing}");
    let note = listing["note"].as_str().unwrap_or_default();
    assert!(note.contains("\"all\":true"), "{listing}");
}

#[then("the listing should hide nothing")]
fn then_listing_hides_nothing(world: &mut QuectoWorld) {
    let listing = last_listing(world);
    for key in ["hidden", "omitted", "note"] {
        assert!(listing.get(key).is_none(), "{key}: {listing}");
    }
}

#[then(expr = "the listing row {string} should be this session's own")]
fn then_row_own(world: &mut QuectoWorld, env_ref: String) {
    let listing = last_listing(world);
    let row = row(&listing, &env_ref);
    assert_eq!(row["own"], true, "{row}");
    assert!(row.get("session").is_none(), "{row}");
}

#[then(expr = "the listing row {string} should come from session {string}")]
fn then_row_foreign(world: &mut QuectoWorld, env_ref: String, session: String) {
    let listing = last_listing(world);
    let row = row(&listing, &env_ref);
    assert_eq!(row["own"], false, "{row}");
    assert_eq!(row["session"], session.as_str(), "{row}");
    assert_eq!(row["restored"], true, "{row}");
}

#[then("every listing row should be compact")]
fn then_rows_compact(world: &mut QuectoWorld) {
    let listing = last_listing(world);
    for row in listing["containers"].as_array().into_iter().flatten() {
        for key in ["ref", "status", "config", "members", "own", "checkout"] {
            assert!(row.get(key).is_some(), "{key} missing: {row}");
        }
        assert!(row["members"].is_u64(), "a member count: {row}");
        for dropped in ["environment_uuid", "workspace"] {
            assert!(row.get(dropped).is_none(), "{dropped}: {row}");
        }
        // Only the listed metadata keys, never the create result's blob.
        for key in row["metadata"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, _)| k)
        {
            assert!(
                ["retained", "cause", "inspect_status", "container"].contains(&key.as_str()),
                "{key}: {row}"
            );
        }
    }
}

// ─── Members and workspace, checked against the registry (#2220) ─────────

#[then(expr = "subagents {string} and {string} should share the same workspace")]
fn then_shared_workspace(world: &mut QuectoWorld, a: String, b: String) {
    // Each agent's workspace comes from its OWN spawn result; the joiner must
    // report the creator's workspace, and the listing must agree.
    let wa = world
        .agent_workspaces
        .get(&a)
        .unwrap_or_else(|| {
            panic!(
                "no captured workspace for {a}: {:?}",
                world.agent_workspaces
            )
        })
        .clone();
    let wb = world
        .agent_workspaces
        .get(&b)
        .unwrap_or_else(|| {
            panic!(
                "no captured workspace for {b}: {:?}",
                world.agent_workspaces
            )
        })
        .clone();
    assert!(!wa.is_empty(), "workspace for {a} must be reported");
    assert_eq!(wa, wb, "{a} and {b} must share one workspace");
    let env_ref = world
        .agent_env_refs
        .get(&a)
        .cloned()
        .expect("captured env ref");
    // The registry records the members' workspace; the listing's checkout
    // (#2220) is where they work — the workspace or a checkout inside it.
    let record = world
        .spawn_tool
        .as_ref()
        .expect("spawn tool")
        .environment_registry()
        .get(&env_ref)
        .unwrap_or_else(|| panic!("the registry should record {env_ref}"));
    assert_eq!(
        record.workspace_path,
        std::path::PathBuf::from(&wa),
        "the recorded workspace must match the members' reported workspace"
    );
    let entry = container_listing_entry(world, &env_ref);
    let checkout = entry["checkout"].as_str().unwrap_or_default();
    assert!(
        std::path::Path::new(checkout).starts_with(&wa),
        "the listing's checkout must lie in the members' workspace {wa}: {entry}"
    );
}

#[then(expr = "subagents {string} and {string} should both be listed as members of {string}")]
fn then_both_listed_as_members(world: &mut QuectoWorld, a: String, b: String, env_ref: String) {
    // The listing counts members (#2220); the registry names them.
    let record = world
        .spawn_tool
        .as_ref()
        .expect("spawn tool")
        .environment_registry()
        .get(&env_ref)
        .unwrap_or_else(|| panic!("the registry should record {env_ref}"));
    let members: Vec<&str> = record.members.iter().map(String::as_str).collect();
    let entry = container_listing_entry(world, &env_ref);
    assert_eq!(
        entry["members"].as_u64(),
        Some(members.len() as u64),
        "the listing's member count must match the registry: {entry}"
    );
    let ua = world.agent_spawn_uuids.get(&a).cloned().unwrap_or_default();
    let ub = world.agent_spawn_uuids.get(&b).cloned().unwrap_or_default();
    assert!(
        !ua.is_empty()
            && !ub.is_empty()
            && members.contains(&ua.as_str())
            && members.contains(&ub.as_str()),
        "both agent UUIDs must be members of {env_ref}: members={members:?} a={ua} b={ub}"
    );
}
