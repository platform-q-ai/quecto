//! Step definitions for the sub-agent inspector's kernel-owned piece (#795):
//! the optional `agent_id` on the `get_messages_tail` command.
//!
//! The TUI panel behaviour (activation, focus machine, navigation, scrolling,
//! no-flash rendering) is covered behaviourally by the `quecto-tui` harness and
//! component tests — those exercise the real render/key paths, which a BDD step
//! in this crate cannot reach. Here we lock the protocol contract the kernel
//! introduces, via a real serde round-trip rather than source inspection.

use super::*;
use quecto::interface::cli::protocol::AgentCommand;

#[given(expr = "a get_messages_tail wire line targeting agent {string}")]
fn given_wire_line(world: &mut QuectoWorld, agent: String) {
    world.stdout = format!(r#"{{"type":"get_messages_tail","count":1,"agent_id":"{agent}"}}"#);
}

#[given("a get_messages_tail wire line with no agent_id")]
fn given_wire_line_no_agent(world: &mut QuectoWorld) {
    world.stdout = r#"{"type":"get_messages_tail","count":1}"#.to_string();
}

#[when("the kernel parses the command")]
fn when_parse_command(world: &mut QuectoWorld) {
    let cmd: AgentCommand =
        serde_json::from_str(&world.stdout).expect("get_messages_tail line should parse");
    // Re-serialize so the assertions inspect the parsed-then-emitted shape,
    // proving the field survives a full round-trip (not just deserialization).
    world.stdout = serde_json::to_value(&cmd).unwrap().to_string();
}

#[then(expr = "the parsed command targets agent {string}")]
fn then_targets_agent(world: &mut QuectoWorld, agent: String) {
    let value: serde_json::Value = serde_json::from_str(&world.stdout).unwrap();
    assert_eq!(
        value.get("agent_id").and_then(|v| v.as_str()),
        Some(agent.as_str()),
        "round-tripped command must preserve agent_id (got {})",
        world.stdout
    );
}

#[then("the parsed command targets no agent")]
fn then_targets_no_agent(world: &mut QuectoWorld) {
    let value: serde_json::Value = serde_json::from_str(&world.stdout).unwrap();
    assert!(
        value.get("agent_id").is_none(),
        "absent agent_id must stay absent (parent-targeted), got {}",
        world.stdout
    );
}
