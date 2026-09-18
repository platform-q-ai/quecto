//! The production wiring of the container-config selection (#2024 S4a):
//! `build_agent_from_config` — the path `quecto agent` takes — hands the
//! spawn tool a selection over the run's OWN configuration selection, so a
//! `spawn container: true` from an agent started in a checkout whose
//! trusted overlay labels a different default lands in THAT config. The
//! rig is the real build (the registry's spawn tool, no fixture launcher)
//! with a fake argv-recording create script; the selection is derived from
//! the CLI context exactly as the `agent` command derives it from its cwd.

use super::super::*;
use super::integration_tests::composed_ctx;
use super::selection_for_test;
use crate::composition::tool_policy::build_tool_policy_persistence;
use crate::domain::tool::ToolResult;
use crate::interface::cli::run_with_output;

struct Rig {
    _dir: tempfile::TempDir,
    base: std::path::PathBuf,
    repo: std::path::PathBuf,
    log: std::path::PathBuf,
}

impl Rig {
    /// A base dir whose global file labels `global` as default, a checkout
    /// with no overlay yet, and one create script both entries point at
    /// that records its config name and own argv, then refuses (the test
    /// asserts the selection, not a readiness handshake).
    fn new() -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let base = dir.path().join("base");
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        let log = base.join("create-calls.jsonl");
        let script = base.join("create.sh");
        std::fs::write(
            &script,
            format!(
                "#!/usr/bin/env bash\nown=\"\"\nfor a in \"$@\"; do [ \"$a\" = \"--\" ] && break; own=\"$own $a\"; done\nprintf '{{\"config\":\"%s\",\"own_argv\":\"%s\"}}\\n' \"${{QUECTO_CONTAINER_CONFIG:-}}\" \"${{own# }}\" >> '{}'\nexit 1\n",
                log.display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let config = serde_json::json!({
            "providers": {"openai": {"api_key": "sk-test"}},
            "container_configs": {
                "global": {"default": true, "create": [script, "--repo", "https://example.test/global"], "cleanup": [script]}
            }
        });
        std::fs::write(
            base.join("config.json"),
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .unwrap();
        Self {
            _dir: dir,
            base,
            repo,
            log,
        }
    }

    fn ctx(&self) -> CliContext {
        CliContext {
            cwd: Some(self.repo.clone()),
            ..composed_ctx(&self.base)
        }
    }

    /// The agent-shaped bind: `quecto config set --local` from the checkout
    /// writes the overlay and records its trust.
    fn bind_repo_default(&self, name: &str, repo_url: &str) {
        let script = self.base.join("create.sh");
        let entry = serde_json::json!({
            "default": true,
            "create": [script, "--repo", repo_url],
            "cleanup": [script],
        });
        let out = run_with_output(
            vec![
                "quecto".into(),
                "config".into(),
                "set".into(),
                "--local".into(),
                format!("container_configs.{name}"),
                entry.to_string(),
            ],
            &self.ctx(),
        );
        assert_eq!(
            out.exit_code, 0,
            "stdout: {}\nstderr: {}",
            out.stdout, out.stderr
        );
    }

    fn create_calls(&self) -> Vec<serde_json::Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

fn flags() -> AgentFlags {
    AgentFlags {
        session_name: None,
        no_session: false,
        message: Some("hi".into()),
        system_prompt: None,
        model_override: None,
        max_iterations: Some(1),
        max_time: None,
        uds_mode: false,
        socket_path: None,
        persist: false,
        disabled_tools: vec![],
        effort: None,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: false,
        swarm_participation: crate::infrastructure::tools::swarm_bridge::Participation::none(),
        workflow_spec_path: None,
        inherited_tool_policy: None,
        parent_id: None,
        spawned: false,
        parent_identity_override: None,
        session_key_override: None,
        cwd_override: None,
        web_fetch_tool_factory: None,
        // The launcher's lifecycle is composed as `main` composes it.
        kill_tool: Some(crate::composition::subagent_termination::install_termination_owners),
        retention: Some(crate::composition::sessions::build_retention_handles),
        catalogue: Some(crate::composition::catalogue::build_catalogue_handles),
        provider_runtime: Some(crate::composition::runtime::build_agent_provider),
        tool_policy_persistence: Some(build_tool_policy_persistence),
        admission_context: None,
        parent_control: None,
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        admission: Some(crate::composition::admission::build_admission_handles),
        container_config_selection: Some(
            crate::composition::container_configs::build_agent_container_config_selection,
        ),
        stdin_is_tty: false,
        environment_registry: None,
    }
}

/// `spawn {"container": true}` through the built agent's registry.
fn spawn_container_true(rig: &Rig, selection: &ConfigSelection) -> ToolResult {
    let mut stderr = String::new();
    let build = build_agent_from_config(&rig.base, selection, &flags(), &mut stderr, None)
        .unwrap_or_else(|| panic!("agent build failed: {stderr}"));
    let args = serde_json::json!({
        "agent_id": "container-child",
        "task": "wait",
        "container": true,
        "read_only": true,
    })
    .to_string();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = match rt.block_on(build.agent.execute_tool_for_tests("spawn", &args)) {
        Ok(result) => result,
        Err(error) => ToolResult {
            content: error.to_string(),
            is_error: true,
            image_blocks: vec![],
            delivery_metadata: None,
        },
    };
    // The launch adapter's monitor and rollback tasks live on this runtime.
    rt.shutdown_timeout(std::time::Duration::from_secs(5));
    result
}

#[test]
fn a_checkouts_trusted_overlay_default_is_the_container_the_real_agent_build_spawns() {
    let rig = Rig::new();
    rig.bind_repo_default("app", "https://example.test/app");
    // The selection the `agent` command derives from its working directory.
    let selection = rig.ctx().config_selection().unwrap();
    let ConfigSelection::Layered(layers) = &selection else {
        panic!("expected the layered selection, got {selection:?}");
    };
    assert_eq!(
        layers.overlay.as_deref(),
        Some(rig.repo.join(".quecto").join("config.json").as_path()),
        "the checkout's overlay is the selection's overlay candidate"
    );
    let result = spawn_container_true(&rig, &selection);
    assert!(
        result.is_error && result.content.contains("script-managed create failed"),
        "the fake create script refuses after recording: {}",
        result.content
    );
    let calls = rig.create_calls();
    assert_eq!(calls.len(), 1, "exactly one create call: {calls:?}");
    assert_eq!(calls[0]["config"], "app", "{calls:?}");
    assert_eq!(
        calls[0]["own_argv"], "--repo https://example.test/app",
        "the overlay entry's own argv reached the script: {calls:?}"
    );
}

#[test]
fn a_checkout_without_an_overlay_spawns_the_global_default_through_the_real_agent_build() {
    let rig = Rig::new();
    let selection = selection_for_test(&rig.base.join("config.json"), false);
    let result = spawn_container_true(&rig, &selection);
    assert!(result.is_error, "{}", result.content);
    let calls = rig.create_calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0]["config"], "global", "{calls:?}");
    assert_eq!(calls[0]["own_argv"], "--repo https://example.test/global");
}
