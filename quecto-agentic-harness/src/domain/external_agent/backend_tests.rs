//! The member backend rule (#2287, O1): `claude_code` is accepted only for
//! swarm workers launched by the coordinator into its own container.

use super::*;
use crate::domain::environment_registry::EnvironmentTarget;

fn config(backend: MemberBackend) -> SubagentConfig {
    SubagentConfig {
        task: None,
        container: ContainerSelection::Local,
        agent_id: Some("w1".into()),
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        effort: None,
        disable_tools: Vec::new(),
        read_only: false,
        backend,
    }
}

const HOST: BackendLaunchContext = BackendLaunchContext {
    launcher_is_swarm_participant: false,
};
const COORDINATOR: BackendLaunchContext = BackendLaunchContext {
    launcher_is_swarm_participant: true,
};

const WORKFLOW_REFUSAL: &str =
    "workflow is unavailable for swarm agents; omit workflow, workflow_guards and workflow_spec";

/// A refusal's reason, which must be a tool refusal (its text alone goes
/// out as the spawn tool's error).
fn refusal_text(error: DomainError) -> String {
    match error {
        DomainError::Tool(reason) => reason,
        other => panic!("a backend refusal is a tool refusal: {other:?}"),
    }
}

#[test]
fn claude_code_is_accepted_only_for_container_swarm_workers() {
    let claude = || config(MemberBackend::ClaudeCode);
    let with = |edit: fn(&mut SubagentConfig)| {
        let mut config = claude();
        edit(&mut config);
        config
    };
    // (case, launcher, request, expected refusal: None = accepted)
    let table: Vec<(&str, BackendLaunchContext, SubagentConfig, Option<&str>)> = vec![
        (
            "coordinator into its own container",
            COORDINATOR,
            claude(),
            None,
        ),
        (
            "host parent",
            HOST,
            claude(),
            Some(CLAUDE_CODE_WORKERS_ONLY),
        ),
        (
            "host parent into a new container",
            HOST,
            with(|c| {
                c.container = ContainerSelection::New {
                    container_config: None,
                    name: None,
                }
            }),
            Some(CLAUDE_CODE_WORKERS_ONLY),
        ),
        (
            "coordinator into a new container",
            COORDINATOR,
            with(|c| {
                c.container = ContainerSelection::New {
                    container_config: None,
                    name: None,
                }
            }),
            Some(CLAUDE_CODE_OWN_CONTAINER_ONLY),
        ),
        (
            "coordinator joining another container",
            COORDINATOR,
            with(|c| {
                c.container = ContainerSelection::Existing {
                    target: EnvironmentTarget::Ref("C2".into()),
                }
            }),
            Some(CLAUDE_CODE_OWN_CONTAINER_ONLY),
        ),
        (
            "workflow on",
            COORDINATOR,
            with(|c| c.workflow = true),
            Some(WORKFLOW_REFUSAL),
        ),
        (
            "workflow guards on",
            COORDINATOR,
            with(|c| c.workflow_guards = true),
            Some(WORKFLOW_REFUSAL),
        ),
        (
            "a bound workflow spec",
            COORDINATOR,
            with(|c| {
                c.workflow_spec = Some(crate::domain::workflow::WorkflowSpec {
                    template: crate::domain::workflow::WorkflowTemplate {
                        id: "t".into(),
                        label: "t".into(),
                        description: "t".into(),
                        when_to_use: None,
                        steps: Vec::new(),
                        guards: Vec::new(),
                    },
                })
            }),
            Some(WORKFLOW_REFUSAL),
        ),
        (
            "effort set",
            COORDINATOR,
            with(|c| c.effort = Some("high".into())),
            Some(CLAUDE_CODE_TAKES_NO_EFFORT),
        ),
    ];
    for (case, context, config, expected) in table {
        let outcome = validate_backend(&config, context).map_err(refusal_text);
        assert_eq!(
            outcome,
            expected.map_or(Ok(()), |r| Err(r.to_string())),
            "{case}"
        );
    }
}

#[test]
fn the_quecto_backend_is_accepted_wherever_a_launch_is() {
    for context in [HOST, COORDINATOR] {
        let mut config = config(MemberBackend::Quecto);
        config.effort = Some("high".into());
        config.workflow = true;
        assert!(validate_backend(&config, context).is_ok());
    }
}

#[test]
fn backend_values_round_trip_their_spellings() {
    assert_eq!(MemberBackend::default(), MemberBackend::Quecto);
    for backend in [MemberBackend::Quecto, MemberBackend::ClaudeCode] {
        assert_eq!(
            MemberBackend::from_flag_value(backend.flag_value()),
            Some(backend)
        );
    }
    assert_eq!(
        MemberBackend::from_spawn_value("claude_code"),
        Some(MemberBackend::ClaudeCode)
    );
    for unknown in ["claude-code", "Claude_Code", "", "codex"] {
        assert_eq!(MemberBackend::from_spawn_value(unknown), None, "{unknown}");
    }
    assert_eq!(MemberBackend::from_flag_value("claude_code"), None);
}
