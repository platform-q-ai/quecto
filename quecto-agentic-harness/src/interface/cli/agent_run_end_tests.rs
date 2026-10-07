//! #2206: a one-shot run that ends the way its owner meant — it finished,
//! or `--max-time` stopped it — settles the children it launched before
//! the process exits: a plain container child's box is cleaned up and its
//! record forgotten. A run whose provider failed settles nothing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::integration_tests::{make_test_agent, test_flags};
use super::{AgentFlags, AgentOutput, run_agent_session};
use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
    mint_environment_uuid,
};
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, SubagentRegistry};
use crate::interface::cli::run_end_fleet::{RunEnd, RunHandles};

/// One launched plain container child, the only member of the environment
/// the run created; its cleanup logs itself to `scripts.log`.
struct Child {
    log: std::path::PathBuf,
    environments: EnvironmentRegistry,
    env_ref: String,
    registry: SubagentRegistry,
}

fn launched_child(dir: &std::path::Path) -> Child {
    let log = dir.join("scripts.log");
    let cleanup = dir.join("cleanup.sh");
    std::fs::write(
        &cleanup,
        format!(
            "#!/usr/bin/env bash\necho \"cleanup $QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'\n",
            log.display()
        ),
    )
    .unwrap();
    // A plain container: its workspace resolves and holds no board.
    std::fs::create_dir_all(dir.join("workspace")).unwrap();
    let environments = EnvironmentRegistry::new();
    let env_ref = environments.mint_ref().unwrap();
    environments.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: "env-oneshot".to_string(),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: dir.join("workspace"),
        repository: String::new(),
        script_name: "default".to_string(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec!["true".to_string()],
        retained_cleanup_argv: vec!["bash".to_string(), cleanup.display().to_string()],
        retained_inspect_argv: vec![],
        members: vec!["child".to_string()],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    });
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let mut entry = SubagentEntry::with_identity(
        crate::domain::ids::AgentUuid::new("child"),
        "child".into(),
        dir.join("never.sock"),
        0,
    );
    entry.launch_generation =
        Some(crate::domain::agents::services::subagent_teardown::LaunchGeneration::new(1));
    entry.environment_registry = Some(environments.clone());
    entry.environment_ref = Some(env_ref.clone());
    registry.lock().unwrap().insert("child".to_string(), entry);
    Child {
        log,
        environments,
        env_ref,
        registry,
    }
}

impl Child {
    fn run(
        &self,
        dir: &std::path::Path,
        agent: AgentLoopImpl,
        flags: &AgentFlags,
    ) -> (i32, String) {
        let retention = crate::composition::sessions::build_retention_handles(dir);
        let handles = RunHandles {
            retention: &retention,
            run_end: RunEnd::compose(
                Some(crate::composition::subagent_teardown::build_run_end_fleet),
                Some(self.registry.clone()),
                Some(
                    crate::infrastructure::tools::harness_lifecycle::new_shared_harness_lifecycle(),
                ),
                false,
            ),
        };
        let (mut stdout, mut stderr) = (String::new(), String::new());
        let mut out = AgentOutput {
            stdout: &mut stdout,
            stderr: &mut stderr,
        };
        let code = run_agent_session(
            dir,
            crate::composition::sessions::build_session_handles,
            agent,
            handles,
            flags,
            &mut out,
        );
        (code, stderr)
    }

    fn assert_ended_for_good(&self) {
        assert_eq!(
            std::fs::read_to_string(&self.log).unwrap().trim(),
            "cleanup env-oneshot"
        );
        assert!(
            self.environments.get(&self.env_ref).is_none(),
            "record forgotten"
        );
    }
}

/// #2206 round 1 (policy): a provider error is not the owner's word — it
/// is closer to a crash, and a crash retains. The failed run settles
/// nothing: its child's box and record stay for the parent-loss path and
/// `container kill` / `gc`.
#[test]
fn a_failed_one_shot_run_keeps_its_plain_container_childs_box() {
    let tmp = tempfile::TempDir::new().unwrap();
    let child = launched_child(tmp.path());
    let flags = test_flags(Some("hello"), Some("-"), None);

    let (code, stderr) = child.run(tmp.path(), make_test_agent(tmp.path()), &flags);

    assert_eq!(code, 1, "the provider fails: {stderr}");
    assert!(!child.log.exists(), "no script ran");
    assert_eq!(
        child
            .environments
            .get(&child.env_ref)
            .map(|record| record.status),
        Some(EnvironmentStatus::Running),
        "the record stays"
    );
    assert_eq!(child.registry.lock().unwrap().len(), 1, "nothing settled");
}

/// An agent whose provider lives at `api_base`.
fn agent_at(dir: &std::path::Path, api_base: &str) -> AgentLoopImpl {
    let config_path = dir.join("provider-config.json");
    std::fs::write(
        &config_path,
        format!(r#"{{"providers":{{"openai":{{"api_key":"sk-test","api_base":"{api_base}"}}}}}}"#),
    )
    .unwrap();
    let config =
        crate::infrastructure::config::Config::load(config_path.to_str().unwrap()).unwrap();
    let provider =
        crate::composition::runtime::build_agent_provider(&config, dir, &reqwest::Client::new())
            .unwrap();
    let workspace = std::path::PathBuf::from(config.workspace_path());
    let sandbox = crate::infrastructure::security::sandbox::Sandbox::new(Some(workspace.clone()));
    let registry = crate::infrastructure::extensions::native::build_official_tool_registry(
        crate::composition::find::build_find_tool(
            Arc::new(workspace.clone()),
            Arc::new(sandbox.clone()),
        ),
        workspace,
        sandbox,
        Default::default(),
    );
    AgentLoopImpl::new(crate::application::agent_loop::AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 100,
        temperature: 0.0,
        retention: None,
        session_key: String::new(),
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
}

/// A provider that accepts the connection and never answers, so the run
/// is still in flight at its `--max-time` deadline.
fn hanging_agent(dir: &std::path::Path) -> (AgentLoopImpl, std::net::TcpListener) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let api_base = format!("http://{}", listener.local_addr().unwrap());
    (agent_at(dir, &api_base), listener)
}

#[test]
fn a_finished_one_shot_run_ends_its_plain_container_child_after_saving() {
    let tmp = tempfile::TempDir::new().unwrap();
    let child = launched_child(tmp.path());
    let server_rt = tokio::runtime::Runtime::new().unwrap();
    let server = server_rt.block_on(wiremock::MockServer::start());
    server_rt.block_on(
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "chatcmpl-run-end",
                    "object": "chat.completion",
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "all done"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
                })),
            )
            .mount(&server),
    );
    let agent = agent_at(tmp.path(), &format!("{}/v1", server.uri()));
    let flags = test_flags(Some("hello"), Some("-"), None);

    let (code, stderr) = child.run(tmp.path(), agent, &flags);

    assert_eq!(code, 0, "the run finished: {stderr}");
    child.assert_ended_for_good();
}

#[test]
fn a_one_shot_run_stopped_at_its_max_time_still_ends_its_plain_container_child() {
    let tmp = tempfile::TempDir::new().unwrap();
    let child = launched_child(tmp.path());
    let (agent, _listener) = hanging_agent(tmp.path());
    let mut flags = test_flags(Some("hello"), Some("-"), None);
    flags.max_time = Some(1);

    let (code, stderr) = child.run(tmp.path(), agent, &flags);

    assert_eq!(code, 2, "stopped at the deadline: {stderr}");
    assert!(stderr.contains("max-time exceeded"), "{stderr}");
    child.assert_ended_for_good();
}

/// #2226 review 2: a one-shot run's prompt is an instruction and every
/// message the run appended is saved with that turn origin, so a parent
/// reading the saved transcript takes the run's answer as its report.
#[test]
fn a_one_shot_run_saves_every_message_as_its_instruction_turn() {
    let tmp = tempfile::TempDir::new().unwrap();
    let child = launched_child(tmp.path());
    let server_rt = tokio::runtime::Runtime::new().unwrap();
    let server = server_rt.block_on(wiremock::MockServer::start());
    server_rt.block_on(
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "chatcmpl-origin", "object": "chat.completion",
                    "choices": [{"index": 0, "finish_reason": "stop",
                        "message": {"role": "assistant", "content": "the answer"}}],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
                })),
            )
            .mount(&server),
    );
    let agent = agent_at(tmp.path(), &format!("{}/v1", server.uri()));
    let flags = test_flags(Some("hello"), Some("origin-2226"), None);

    let (code, stderr) = child.run(tmp.path(), agent, &flags);

    assert_eq!(code, 0, "the run finished: {stderr}");
    let saved: Vec<serde_json::Value> = std::fs::read_dir(tmp.path().join("sessions"))
        .unwrap()
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .flat_map(|text| text.lines().map(str::to_owned).collect::<Vec<_>>())
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(&line).ok())
        .flat_map(|record| record["messages"].as_array().cloned().unwrap_or_default())
        .collect();
    let origins: Vec<&serde_json::Value> = saved.iter().map(|m| &m["turn_origin"]).collect();
    assert_eq!(saved.len(), 2, "the prompt and the answer: {saved:?}");
    assert!(
        origins.iter().all(|origin| *origin == "instruction"),
        "{saved:?}"
    );
}
