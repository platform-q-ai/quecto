//! Steps for `models_refresh.feature` (#1846): the UDS `refresh_models`
//! reply as the presenter renders the composed refresh use case's report.

use super::uds_steps::{find_agent_response, find_agent_response_by_id};
use super::*;

#[when(expr = "I send refresh_models for source {string} with id {string}")]
fn when_send_refresh_models_for(world: &mut QuectoWorld, source: String, id: String) {
    let cmd = serde_json::json!({"type": "refresh_models", "id": id, "source": source});
    world
        .uds_commands
        .push(serde_json::to_string(&cmd).expect("serialize command"));
}

fn refresh_outcome(world: &QuectoWorld, source: &str) -> serde_json::Value {
    let resp = find_agent_response(world, "refresh_models").expect("no refresh_models response");
    resp["data"]["outcomes"]
        .as_array()
        .expect("outcomes array")
        .iter()
        .find(|o| o["source"] == source)
        .cloned()
        .unwrap_or_else(|| panic!("no outcome for {source}: {resp}"))
}

#[then(expr = "the refresh_models response reports source {string} as {string} with {int} models")]
fn then_refresh_outcome_with_models(
    world: &mut QuectoWorld,
    source: String,
    status: String,
    models: u64,
) {
    let outcome = refresh_outcome(world, &source);
    assert_eq!(outcome["status"], status, "{outcome}");
    assert_eq!(outcome["models"], models, "{outcome}");
}

#[then(expr = "the refresh_models response reports source {string} as {string}")]
fn then_refresh_outcome(world: &mut QuectoWorld, source: String, status: String) {
    let outcome = refresh_outcome(world, &source);
    assert_eq!(outcome["status"], status, "{outcome}");
}

#[then("the refresh_models response carries a published generation")]
fn then_refresh_generation(world: &mut QuectoWorld) {
    let resp = find_agent_response(world, "refresh_models").expect("no refresh_models response");
    assert!(resp["data"]["generation"].is_u64(), "{resp}");
}

/// `list_models` re-resolves (one generation) after the refresh published:
/// exact adjacency proves the refresh is the publish before it.
#[then("the list_models generation is exactly one after the refresh_models generation")]
fn then_list_follows_refresh_generation(world: &mut QuectoWorld) {
    let refresh = find_agent_response(world, "refresh_models").expect("no refresh_models response");
    let list = find_agent_response(world, "list_models").expect("no list_models response");
    let refreshed = refresh["data"]["generation"]
        .as_u64()
        .expect("refresh generation");
    let listed = list["data"]["generation"]
        .as_u64()
        .expect("list generation");
    assert_eq!(listed, refreshed + 1, "refresh: {refresh}\nlist: {list}");
}

/// The published generation moves by one per `list_models` and by one per
/// runtime rebuild, so the delta between two listings counts the rebuilds
/// between them (#1849, #2022 review F4).
#[then(
    expr = "the list_models generation reported by {string} is exactly {int} after the one reported by {string}"
)]
fn then_listing_generation_delta(
    world: &mut QuectoWorld,
    later_id: String,
    delta: u64,
    earlier_id: String,
) {
    let generation_of = |id: &str| -> u64 {
        let resp = find_agent_response_by_id(world, id).unwrap_or_else(|| {
            panic!(
                "no response with id {id:?}\nevents: {:#?}",
                world.agent_events
            )
        });
        assert_eq!(resp["command"], "list_models", "{resp}");
        resp["data"]["generation"]
            .as_u64()
            .unwrap_or_else(|| panic!("no generation on {id:?}: {resp}"))
    };
    let earlier = generation_of(&earlier_id);
    let later = generation_of(&later_id);
    assert_eq!(
        later,
        earlier + delta,
        "expected {later_id:?} to be exactly {delta} generation(s) after {earlier_id:?} (earlier={earlier}, later={later})\nevents: {:#?}",
        world.agent_events
    );
}

#[then("the refresh_models response carries no published generation")]
fn then_refresh_no_generation(world: &mut QuectoWorld) {
    let resp = find_agent_response(world, "refresh_models").expect("no refresh_models response");
    assert!(resp["data"]["generation"].is_null(), "{resp}");
}
