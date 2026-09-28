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
    use crate::application::tools::ports::Tool;
    let error = swarm_tool(true)
        .execute(r#"{"agent_id":"w","workflow":true}"#)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with("tool error: workflow is unavailable for swarm agents"),
        "{error}"
    );
}

/// #2287 (O1): `backend` picks the child's brain; `claude_code` is
/// accepted only under the domain's backend rule, with its exact refusal.
/// Serial: the rule reads `QUECTO_RUNTIME_CONFIG_PATH`, which other tests
/// set.
#[test]
#[serial_test::serial]
fn the_backend_parameter_selects_the_brain_under_the_backend_rule() {
    use crate::application::tools::ports::Tool;
    use crate::domain::external_agent::backend::{CLAUDE_CODE_WORKERS_ONLY, MemberBackend};
    let worker = swarm_tool(true)
        .parse_args(r#"{"agent_id":"w1","backend":"claude_code"}"#)
        .unwrap();
    assert_eq!(worker.backend, MemberBackend::ClaudeCode);
    for input in [r#"{}"#, r#"{"backend":"quecto"}"#, r#"{"backend":null}"#] {
        for participating in [false, true] {
            let config = swarm_tool(participating).parse_args(input).unwrap();
            assert_eq!(config.backend, MemberBackend::Quecto, "{input}");
        }
    }
    assert_eq!(
        SpawnTool::new(vec![])
            .parse_args(r#"{"backend":"claude_code"}"#)
            .unwrap_err(),
        CLAUDE_CODE_WORKERS_ONLY
    );
    for input in [
        r#"{"backend":"codex"}"#,
        r#"{"backend":"claude-code"}"#,
        r#"{"backend":1}"#,
    ] {
        assert_eq!(
            swarm_tool(true).parse_args(input).unwrap_err(),
            "backend must be one of: quecto, claude_code",
            "{input}"
        );
    }
    // #2287 review (M3): not advertised until S4 (#2288) serves the
    // member; S4 adds the property back to the schema.
    let definition = SpawnTool::new(vec![]).definition();
    let schema: serde_json::Value = serde_json::from_str(&definition.parameters_schema).unwrap();
    assert!(
        schema["properties"].get("backend").is_none(),
        "{}",
        definition.parameters_schema
    );
    assert!(!definition.parameters_schema.contains("claude_code"));
    assert!(!definition.description.contains("backend"));
    assert!(!definition.description.contains("claude_code"));
}

/// Runs `body` with `QUECTO_RUNTIME_CONFIG_PATH` set to `value` (unset for
/// `None`), restoring it after. Callers are `#[serial_test::serial]`.
fn with_runtime_config(value: Option<&str>, body: impl FnOnce()) {
    let old = std::env::var_os("QUECTO_RUNTIME_CONFIG_PATH");
    // SAFETY: serialized by #[serial_test::serial]; restored below.
    unsafe {
        match value {
            Some(value) => std::env::set_var("QUECTO_RUNTIME_CONFIG_PATH", value),
            None => std::env::remove_var("QUECTO_RUNTIME_CONFIG_PATH"),
        }
    }
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    // SAFETY: as above.
    unsafe {
        match old {
            Some(old) => std::env::set_var("QUECTO_RUNTIME_CONFIG_PATH", old),
            None => std::env::remove_var("QUECTO_RUNTIME_CONFIG_PATH"),
        }
    }
    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}

/// #2287 review round 2 (M3): the launch hands a child more than its
/// `SubagentConfig`: the inherited tool policy
/// (`--inherited-tool-policy-snapshot`) and a forwarded `--config`. A
/// claude worker would honour neither, so a coordinator under either is
/// refused `claude_code` (#957), and still launches quecto children.
#[test]
#[serial_test::serial]
fn claude_code_is_refused_to_a_launcher_whose_restrictions_it_would_drop() {
    use crate::domain::external_agent::backend::{
        CLAUDE_CODE_NO_FORWARDED_CONFIG, CLAUDE_CODE_NO_INHERITED_TOOL_POLICY,
    };
    let claude = r#"{"agent_id":"w1","backend":"claude_code"}"#;
    with_runtime_config(None, || {
        let restricted = swarm_tool(true);
        super::super::spawn_inherited_policy::set_from_tools(
            &restricted.inherited_tool_policy,
            std::collections::BTreeMap::new(),
        );
        assert_eq!(
            restricted.parse_args(claude).unwrap_err(),
            CLAUDE_CODE_NO_INHERITED_TOOL_POLICY
        );
        restricted
            .parse_args(r#"{"agent_id":"w1"}"#)
            .expect("a quecto child takes the policy");
        swarm_tool(true)
            .parse_args(claude)
            .expect("an unrestricted coordinator launches a claude worker");
    });
    with_runtime_config(Some("/run/quecto/global.toml"), || {
        assert_eq!(
            swarm_tool(true).parse_args(claude).unwrap_err(),
            CLAUDE_CODE_NO_FORWARDED_CONFIG
        );
        swarm_tool(true)
            .parse_args(r#"{"agent_id":"w1"}"#)
            .expect("a quecto child takes the config");
    });
}
