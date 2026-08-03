use super::*;
use quecto::domain::tool::ToolPolicyChildPropagator;
use quecto::infrastructure::tools::subagent_policy_gateway::SubagentPolicyGateway;

#[then("the live policy propagation gateway should report children")]
fn then_live_policy_gateway_reports_children(world: &mut QuectoWorld) {
    let gateway = SubagentPolicyGateway::new(world.subagent_protocol_registry.clone());

    assert!(gateway.has_children());
}

#[then("the live policy propagation gateway should report no children")]
fn then_live_policy_gateway_reports_no_children(world: &mut QuectoWorld) {
    let gateway = SubagentPolicyGateway::new(world.subagent_protocol_registry.clone());

    assert!(!gateway.has_children());
}
