//! Test-only production composition and process-isolated launch cases for #2708.
use super::*;

pub(crate) struct Composed {
    pub(crate) registry: crate::infrastructure::tools::registry::ToolRegistryImpl,
    pub(crate) runtime: Option<ParentOutcome>,
    pub(crate) workflow: Option<crate::interface::cli::controllers::shared::WorkflowStateHandle>,
    pub(crate) subagents:
        Option<crate::infrastructure::tools::agents::subagent_registry::SubagentRegistry>,
    pub(crate) snippets: String,
    pub(crate) guidance_append: Option<crate::application::agent_turn::agent_loop::GuidanceAppend>,
    // Lifecycle/reload fixtures retain the production registry's scratch home.
    pub(crate) _home: tempfile::TempDir,
}

/// Run each launch in a new process: never mutate the test runner's environment.
/// The timeout also bounds any accidental automatic-turn/wait regression.
pub(crate) fn isolated(test: &str, switches: &[&str]) -> bool {
    if std::env::var("QUECTO_2708_TEST_CASE").ok().as_deref() == Some(test) {
        return true;
    }
    let mut failures = Vec::new();
    for switch in switches {
        for root in ["default", "named", "ephemeral"] {
            let mut command = std::process::Command::new("timeout");
            command.args(["--signal=KILL", "60s"]);
            command.arg(std::env::current_exe().unwrap());
            let (_, selection) = test.split_once("::").expect("crate-qualified test name");
            command.args(["--exact", selection, "--nocapture"]);
            command.env("QUECTO_2708_TEST_CASE", test);
            command.env("QUECTO_2708_TEST_ROOT", root);
            command.env_remove("QUECTO_TURN_OUTCOMES");
            if matches!(*switch, "1" | "other") {
                command.env("QUECTO_TURN_OUTCOMES", switch);
            }
            let output = command.output().expect("isolated test process");
            println!(
                "launch root={root} switch={switch}: {}\n{}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("running 1 test\n"),
                "exact child selection must be nonzero: {selection}"
            );
            match output.status.code() {
                Some(0) => {}
                _ => failures.push(format!("{root}/{switch}: {}", output.status)),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "production activation cases failed: {failures:?}"
    );
    false
}

/// Same builder injected by main; real launch authority and startup policy gates.
/// No directly supplied runtime and no manually registered declaration tool.
pub(crate) fn compose(uds: bool) -> Composed {
    compose_with(uds, |_| {})
}

pub(crate) fn compose_with(uds: bool, apply: impl FnOnce(&mut AgentFlags)) -> Composed {
    compose_with_config(uds, apply, Config::default())
}

pub(crate) fn compose_with_config(
    uds: bool,
    apply: impl FnOnce(&mut AgentFlags),
    config: Config,
) -> Composed {
    let mut flags =
        parse_agent_flags(&["-m".into(), "test".into()], &mut String::new()).expect("launch flags");
    let tmp = match std::env::var_os("RUN16_SCRATCH") {
        Some(parent) => tempfile::TempDir::new_in(parent).unwrap(),
        None => tempfile::TempDir::new().unwrap(),
    };
    let mut ctx = crate::interface::cli::mod_tests::active_runtime_2708_context();
    ctx.cwd = Some(tmp.path().to_path_buf());
    flags.adopt_context(&ctx);
    flags.uds_mode = uds;
    flags.workflow_disabled = true;
    flags.no_session = false;
    flags.session_name = None;
    match std::env::var("QUECTO_2708_TEST_ROOT").unwrap().as_str() {
        "default" => {}
        "named" => flags.session_name = Some("activation-root".into()),
        "ephemeral" => flags.no_session = true,
        root => panic!("unexpected root {root}"),
    }
    apply(&mut flags);
    let http = reqwest::Client::new();
    let mut stderr = String::new();
    let built = build_tool_registry(ToolRegistryArgs {
        base_dir: tmp.path(),
        config_path: tmp.path(),
        config: &config,
        http_client: &http,
        web_fetch_tool: None,
        flags: &flags,
        stderr: &mut stderr,
        broadcast_tx: None,
        cwd: tmp.path(),
        home_dir: Some(tmp.path()),
        effort_control: crate::composition::catalogue::build_catalogue_handles(tmp.path(), None)
            .effort,
        container_configs: crate::composition::container_configs::build_container_config_handles(
            tmp.path(),
            None,
        ),
    })
    .unwrap_or_else(|error| panic!("production registry: {error}; {stderr}"));
    let guarded = built.parent_outcome.is_some();
    assert_eq!(
        built
            .registry
            .definitions_for(
                crate::domain::tool_policy::value_objects::tool::ToolProfileContext::Parent
            )
            .iter()
            .any(|tool| tool.name == "declare_turn_outcome"),
        guarded
    );
    assert_eq!(
        built
            .extension_prompt_snippets
            .contains(crate::domain::agents::value_objects::turn_outcome::DECLARE_GUIDANCE),
        guarded
    );
    Composed {
        registry: built.registry,
        runtime: built.parent_outcome,
        workflow: built.workflow_state,
        subagents: built.subagent_registry,
        snippets: built.extension_prompt_snippets,
        guidance_append: built.guidance_append,
        _home: tmp,
    }
}

#[test]
fn active_runtime_2708_characterization_excluded_roles_and_policy() {
    let test = concat!(
        module_path!(),
        "::active_runtime_2708_characterization_excluded_roles_and_policy"
    );
    if isolated(test, &["unset", "1", "other"]) {
        type Fact = fn(&mut AgentFlags);
        let excluded: [(&str, Fact); 8] = [
            ("spawned", |f| f.spawned = true),
            ("parent-id", |f| f.parent_id = Some("parent".into())),
            ("independent coordinator", |f| f.coordinator = true),
            ("child coordinator", |f| {
                f.coordinator = true;
                f.spawned = true;
            }),
            ("parent control", |f| {
                f.parent_control = Some("/nonexistent/control".into())
            }),
            ("swarm", |f| {
                f.swarm_participation =
                    crate::infrastructure::tools::swarm::swarm_bridge::Participation::Fixed(true)
            }),
            ("withheld", |f| {
                f.disabled_tools = vec!["declare_turn_outcome".into()]
            }),
            ("uncomposed legacy", |f| f.parent_outcome = None),
        ];
        for uds in [false, true] {
            for (name, apply) in excluded {
                assert!(
                    compose_with(uds, apply).runtime.is_none(),
                    "{name}, uds={uds}"
                );
            }
        }
    }
}
