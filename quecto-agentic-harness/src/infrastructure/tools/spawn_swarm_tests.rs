use super::*;

#[test]
fn workflow_container_launch_is_rejected_before_effects() {
    let tool = SpawnTool::new(vec![]);
    for container in [
        serde_json::json!(true),
        serde_json::json!({"mode":"existing","ref":"C1"}),
    ] {
        let error = tool
            .parse_args(&serde_json::json!({"container":container,"workflow":true}).to_string())
            .unwrap_err();
        assert!(
            error.contains("workflow is unavailable for swarm agents"),
            "{error}"
        );
    }
    assert!(
        tool.parse_args(r#"{"workflow":true}"#).is_ok(),
        "host local workflow remains supported"
    );
}

#[test]
fn swarm_worker_rejects_every_workflow_activation_form() {
    let tool =
        SpawnTool::new(vec![]).with_swarm_context(Some(super::super::swarm_bridge::SwarmContext {
            checkout: std::env::temp_dir(),
            member: "worker".into(),
            lifecycle: Arc::new(crate::application::swarm::LifecycleService),
        }));
    for input in [
        r#"{"workflow":true}"#,
        r#"{"workflow_guards":true}"#,
        r#"{"workflow_spec":{}}"#,
    ] {
        assert!(
            tool.parse_args(input)
                .unwrap_err()
                .contains("workflow is unavailable for swarm agents")
        );
    }
    assert!(
        tool.parse_args(r#"{"workflow":false,"workflow_spec":null}"#)
            .is_ok()
    );
}
