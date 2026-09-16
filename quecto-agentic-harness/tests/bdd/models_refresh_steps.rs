//! Steps for `models_refresh.feature` (#1846): the UDS `refresh_models`
//! reply as the presenter renders the composed refresh use case's report.

use super::uds_steps::find_agent_response;
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

#[then("the refresh_models response carries no published generation")]
fn then_refresh_no_generation(world: &mut QuectoWorld) {
    let resp = find_agent_response(world, "refresh_models").expect("no refresh_models response");
    assert!(resp["data"]["generation"].is_null(), "{resp}");
}
