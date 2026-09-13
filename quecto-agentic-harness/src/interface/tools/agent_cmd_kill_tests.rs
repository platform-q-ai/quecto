use super::*;
use crate::application::subagents::dto::TerminationResult;
use crate::application::subagents::ports::ResolutionError;
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{
    DelegatedAgentIdentity, LaunchGeneration, TerminationRouteError,
};

fn outcome(result: TerminationResult, removed: usize) -> KillDelegatedAgentOutcome {
    KillDelegatedAgentOutcome {
        target: DelegatedAgentIdentity::new("A", LaunchGeneration::new(1)),
        result,
        removed: (0..removed)
            .map(|n| AgentUuid::new(format!("r{n}")))
            .collect(),
    }
}

#[test]
fn parsing_accepts_only_the_delivery_shape() {
    assert_eq!(
        parse_kill_arguments(r#"{"agent_id":"w1","command":"kill"}"#)
            .unwrap()
            .reference,
        "w1"
    );
    assert!(
        parse_kill_arguments("not json")
            .unwrap_err()
            .contains("invalid JSON")
    );
    assert_eq!(
        parse_kill_arguments(r#"{"command":"kill"}"#).unwrap_err(),
        "missing required field: agent_id"
    );
    assert_eq!(
        parse_kill_arguments(r#"{"agent_id":"","command":"kill"}"#).unwrap_err(),
        "agent_id must be 1-64 characters"
    );
    assert_eq!(
        parse_kill_arguments(r#"{"agent_id":"a b","command":"kill"}"#).unwrap_err(),
        "agent_id must use only [a-zA-Z0-9_-]"
    );
    let long = "x".repeat(65);
    assert!(parse_kill_arguments(&format!(r#"{{"agent_id":"{long}"}}"#)).is_err());
}

#[test]
fn outcomes_present_the_result_vocabulary_and_cap_the_list() {
    let presented = present_outcome(&outcome(TerminationResult::Graceful, 2));
    assert!(!presented.is_error);
    let body: serde_json::Value = serde_json::from_str(&presented.content).unwrap();
    assert_eq!(body["result"], "graceful");
    assert_eq!(body["target"], "A");
    assert_eq!(body["killed"], serde_json::json!(["r0", "r1"]));
    assert!(body.get("omitted_agents").is_none());
    assert!(body.get("signalled").is_none(), "#1928 vocabulary is gone");

    let capped = present_outcome(&outcome(TerminationResult::Fallback, 23));
    let body: serde_json::Value = serde_json::from_str(&capped.content).unwrap();
    assert_eq!(body["result"], "fallback");
    assert_eq!(body["killed"].as_array().unwrap().len(), 20);
    assert_eq!(body["omitted_agents"], 3);

    let exited = present_outcome(&outcome(TerminationResult::AlreadyExited, 1));
    assert!(exited.content.contains("already-exited"));
}

#[test]
fn errors_present_failed_as_a_result_and_refusals_as_agent_cmd_errors() {
    let failed = present_error(
        "w1",
        &KillDelegatedAgentError::Failed {
            detail: "no exit".into(),
        },
    );
    assert!(failed.is_error);
    let body: serde_json::Value = serde_json::from_str(&failed.content).unwrap();
    assert_eq!(body["result"], "failed");
    assert_eq!(body["target"], "w1");
    assert_eq!(body["error"], "no exit");

    let unknown = present_error(
        "ghost",
        &KillDelegatedAgentError::Unresolved(ResolutionError::Unknown),
    );
    assert!(unknown.is_error);
    assert_eq!(
        unknown.content,
        "agent_cmd error: subagent 'ghost' not found in registry"
    );
    for error in [
        KillDelegatedAgentError::AlreadyStopping,
        KillDelegatedAgentError::NotAccepting,
        KillDelegatedAgentError::Rejected(TerminationRouteError::TargetIsSelf),
        KillDelegatedAgentError::RouteUnreachable {
            via: AgentUuid::new("A"),
            detail: "gone".into(),
        },
    ] {
        let presented = present_error("w1", &error);
        assert!(presented.is_error);
        assert!(presented.content.starts_with("agent_cmd error: "));
        assert!(presented.content.contains(&error.to_string()));
    }
}

#[tokio::test]
async fn the_tool_parses_invokes_and_presents() {
    use crate::application::subagents::ports::TerminationConclusion;
    use crate::application::subagents::use_cases::lifecycle_fakes::{
        FakeCompensation, FakeRegistry, FakeTermination,
    };
    use crate::application::subagents::use_cases::teardown_fakes::{
        FakeLifecycle, FakeRouting, root_tree,
    };
    use crate::application::subagents::use_cases::{
        KillDelegatedAgent, KillDelegatedAgentPorts, TerminateDelegatedAgent,
    };
    use crate::domain::tool::Tool;

    let registry = FakeRegistry::new().with_row("A", 1, "alpha");
    let lifecycle = FakeLifecycle::new(root_tree());
    let routing = FakeRouting::new();
    let use_case = Arc::new(KillDelegatedAgent::new(
        Arc::new(TerminateDelegatedAgent::new(lifecycle.clone(), routing)),
        KillDelegatedAgentPorts {
            registry: registry.clone(),
            lifecycle,
            termination: FakeTermination::new(
                registry.clone(),
                TerminationConclusion::ExitedAfterProtocol,
            ),
            compensation: FakeCompensation::new(registry),
        },
    ));
    let tool = KillDelegatedAgentTool::new(use_case);
    assert_eq!(tool.definition().name, KILL_TOOL_NAME);
    let bad = tool.execute(r#"{"command":"kill"}"#).await.unwrap();
    assert!(bad.is_error);
    assert!(bad.content.contains("missing required field"));
    let ok = tool
        .execute(r#"{"agent_id":"alpha","command":"kill"}"#)
        .await
        .unwrap();
    assert!(!ok.is_error, "{}", ok.content);
    assert!(ok.content.contains("\"result\":\"graceful\""));
    let missing = tool
        .execute(r#"{"agent_id":"ghost","command":"kill"}"#)
        .await
        .unwrap();
    assert!(missing.is_error);
    assert!(missing.content.contains("not found"));
}
