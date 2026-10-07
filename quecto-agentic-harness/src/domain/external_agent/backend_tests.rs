//! The member backend rule (#2287, O1): `claude_code` is accepted only for
//! swarm workers launched by the coordinator into its own container.

use super::*;
use crate::domain::environments::entities::environment_registry::EnvironmentTarget;

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
        coordinator: false,
    }
}

const HOST: BackendLaunchContext = BackendLaunchContext {
    launcher_is_swarm_participant: false,
    inherited_tool_policy: false,
    forwards_config: false,
};
const COORDINATOR: BackendLaunchContext = BackendLaunchContext {
    launcher_is_swarm_participant: true,
    inherited_tool_policy: false,
    forwards_config: false,
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

/// #2287 review (L2): a `claude_code` launch takes only the fields it
/// honours; each one it would ignore is refused with its own text.
#[test]
fn claude_code_refuses_every_field_it_would_ignore() {
    let claude = || config(MemberBackend::ClaudeCode);
    let with = |edit: fn(&mut SubagentConfig)| {
        let mut config = claude();
        edit(&mut config);
        config
    };
    let refused: Vec<(&str, SubagentConfig, &str)> = vec![
        (
            "read_only",
            with(|c| c.read_only = true),
            "backend claude_code takes no read_only; omit read_only",
        ),
        (
            "disable_tools",
            with(|c| c.disable_tools = vec!["write".into()]),
            "backend claude_code takes no disable_tools; omit disable_tools",
        ),
        (
            "system",
            with(|c| c.system = Some("be terse".into())),
            "backend claude_code takes no system prompt; omit system",
        ),
        (
            "config",
            with(|c| c.config_path = Some("/work/quecto.toml".into())),
            "backend claude_code takes no config; omit config",
        ),
        (
            "another provider's model",
            with(|c| c.model = Some("openai/gpt-6.1-sol".into())),
            "backend claude_code runs anthropic models only; omit model or name an anthropic/ model",
        ),
        (
            "a model without its provider",
            with(|c| c.model = Some("claude-sonnet-5".into())),
            "backend claude_code runs anthropic models only; omit model or name an anthropic/ model",
        ),
    ];
    let mut texts = std::collections::BTreeSet::new();
    for (case, request, expected) in refused {
        let error = validate_backend(&request, COORDINATOR).expect_err(case);
        assert_eq!(refusal_text(error), expected, "{case}");
        texts.insert(expected);
    }
    assert_eq!(texts.len(), 5, "each field has its own refusal");
    for (constant, text) in [
        (CLAUDE_CODE_TAKES_NO_READ_ONLY, "read_only"),
        (CLAUDE_CODE_TAKES_NO_DISABLE_TOOLS, "disable_tools"),
        (CLAUDE_CODE_TAKES_NO_SYSTEM, "system"),
        (CLAUDE_CODE_TAKES_NO_CONFIG, "config"),
        (CLAUDE_CODE_ANTHROPIC_MODELS_ONLY, "anthropic/"),
    ] {
        assert!(
            texts.contains(constant) && constant.contains(text),
            "{constant}"
        );
    }

    // What it honours: a task, a label and an anthropic model.
    let accepted = with(|c| {
        c.task = Some("do the work".into());
        c.model = Some("anthropic/claude-sonnet-5".into());
    });
    validate_backend(&accepted, COORDINATOR).expect("a task, a label and an anthropic model");
    // quecto takes every field, as before.
    let mut quecto = with(|c| {
        c.read_only = true;
        c.system = Some("s".into());
        c.model = Some("openai/gpt-6.1-sol".into());
    });
    quecto.backend = MemberBackend::Quecto;
    validate_backend(&quecto, HOST).expect("quecto takes every field");
}

/// #2287 review round 2 (M3): what the launch forwards outside
/// `SubagentConfig` is part of the rule. A launcher under an inherited
/// tool policy, or one whose config the launch would forward, could
/// otherwise start a claude worker free of its restrictions (#957).
#[test]
fn claude_code_refuses_a_launcher_whose_restrictions_it_would_drop() {
    let claude = config(MemberBackend::ClaudeCode);
    let restricted = BackendLaunchContext {
        inherited_tool_policy: true,
        ..COORDINATOR
    };
    assert_eq!(
        refusal_text(validate_backend(&claude, restricted).expect_err("inherited policy")),
        "backend claude_code cannot honour the tool policy this agent inherited; a restricted \
         agent launches only backend quecto"
    );
    assert_eq!(
        refusal_text(validate_backend(&claude, restricted).unwrap_err()),
        CLAUDE_CODE_NO_INHERITED_TOOL_POLICY
    );
    let configured = BackendLaunchContext {
        forwards_config: true,
        ..COORDINATOR
    };
    assert_eq!(
        refusal_text(validate_backend(&claude, configured).expect_err("forwarded config")),
        "backend claude_code cannot honour the config this agent runs under and would forward; \
         an agent under a config launches only backend quecto"
    );
    assert_eq!(
        refusal_text(validate_backend(&claude, configured).unwrap_err()),
        CLAUDE_CODE_NO_FORWARDED_CONFIG
    );
    // quecto children take both, as before.
    let quecto = config(MemberBackend::Quecto);
    for context in [restricted, configured] {
        validate_backend(&quecto, context).expect("quecto honours both");
    }
    validate_backend(&claude, COORDINATOR).expect("an unrestricted coordinator");
}
