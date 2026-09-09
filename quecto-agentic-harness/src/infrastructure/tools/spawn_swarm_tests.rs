use super::super::swarm_bridge::Participation;
use super::*;

fn swarm_tool(participating: bool) -> SpawnTool {
    SpawnTool::new(vec![])
        .with_swarm_context(Some(super::super::swarm_bridge::SwarmContext {
            checkout: std::env::temp_dir(),
            member: "worker".into(),
            lifecycle: Arc::new(crate::application::swarm::LifecycleService),
        }))
        .with_swarm_participation(Participation::Fixed(participating))
}

/// #1715: workflow eligibility follows swarm participation, not
/// containerization. A host parent may launch or join a container with a
/// workflow; only swarm-local worker launches are refused.
#[test]
fn ordinary_container_launches_keep_workflow_eligibility() {
    let tool = SpawnTool::new(vec![]);
    for container in [
        serde_json::json!(true),
        serde_json::json!({"mode":"new","container_config":"default","name":"env"}),
        serde_json::json!({"mode":"existing","ref":"C1"}),
    ] {
        for activation in [
            serde_json::json!({"workflow":true}),
            serde_json::json!({"workflow":true,"workflow_guards":true}),
        ] {
            let mut args = activation.clone();
            args["container"] = container.clone();
            let config = tool
                .parse_args(&args.to_string())
                .unwrap_or_else(|e| panic!("{args}: {e}"));
            assert!(
                config.workflow || config.workflow_guards || config.workflow_spec.is_some(),
                "workflow request survives validation: {args}"
            );
        }
    }
    assert!(
        tool.parse_args(r#"{"workflow":true}"#).is_ok(),
        "host local workflow remains supported"
    );
    // A container agent whose run is still the bootstrap placeholder is an
    // ordinary container: its local children may run workflows too.
    assert!(swarm_tool(false).parse_args(r#"{"workflow":true}"#).is_ok());
}

#[test]
fn swarm_worker_rejects_every_workflow_activation_form() {
    let tool = swarm_tool(true);
    for input in [
        r#"{"workflow":true}"#,
        r#"{"workflow_guards":true}"#,
        r#"{"workflow_spec":{}}"#,
        r#"{"container":true,"workflow":true}"#,
    ] {
        assert!(
            tool.parse_args(input)
                .unwrap_err()
                .contains("workflow is unavailable for swarm agents"),
            "{input}"
        );
    }
    assert!(
        tool.parse_args(r#"{"workflow":false,"workflow_spec":null}"#)
            .is_ok()
    );
}

/// The launch path re-validates with the same rule, so a config built
/// elsewhere cannot slip a workflow into a swarm worker.
#[tokio::test]
async fn swarm_worker_launch_revalidates_workflow_before_effects() {
    use crate::domain::tool::Tool;
    let result = swarm_tool(true)
        .execute(r#"{"agent_id":"w","workflow":true}"#)
        .await
        .unwrap();
    assert!(result.is_error, "{}", result.content);
    assert!(
        result
            .content
            .contains("workflow is unavailable for swarm agents")
    );
}
