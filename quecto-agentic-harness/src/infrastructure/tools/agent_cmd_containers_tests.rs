use std::sync::Arc;

use super::*;
use crate::application::environments::ports::{
    EnvironmentMemberShutdown, MemberShutdownReport, MemberShutdownResult, PortFuture,
    SettledMember,
};
use crate::application::environments::use_cases::{KillEnvironment, ListEnvironmentsQuery};
use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, mint_environment_uuid,
};

/// Settles every member gracefully without asking anyone: the adapter's
/// presentation is what these tests pin.
struct SettleAll;

impl EnvironmentMemberShutdown for SettleAll {
    fn shutdown_members<'a>(
        &'a self,
        members: &'a [String],
    ) -> PortFuture<'a, MemberShutdownReport> {
        Box::pin(async move {
            MemberShutdownReport {
                settled: members
                    .iter()
                    .map(|member| SettledMember {
                        member: member.clone(),
                        result: MemberShutdownResult::Graceful,
                    })
                    .collect(),
                unsettled: Vec::new(),
            }
        })
    }
}

fn use_case(registry: EnvironmentRegistry) -> Arc<KillEnvironment> {
    Arc::new(KillEnvironment::new(
        registry,
        Arc::new(SettleAll),
        Arc::new(super::super::environment_commands::ScriptEnvironmentCommands::default()),
    ))
}

fn committed_registry() -> EnvironmentRegistry {
    let registry = EnvironmentRegistry::new();
    let env_ref = registry.mint_ref();
    registry.commit(EnvironmentRecord {
        environment_ref: env_ref,
        environment_id: "env-tool".into(),
        environment_uuid: mint_environment_uuid(),
        name: Some("tool-env".into()),
        workspace_path: std::path::PathBuf::from("/ws/tool"),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec!["true".into()],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec!["impl-1517".into(), "rev-a".into()],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: crate::domain::environment_registry::EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    });
    registry
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

#[test]
fn container_commands_are_recognized() {
    assert!(is_container_command(
        &serde_json::json!({"command":"get_containers"})
    ));
    assert!(is_container_command(
        &serde_json::json!({"command":"kill_container"})
    ));
    assert!(!is_container_command(
        &serde_json::json!({"command":"kill"})
    ));
}

#[test]
fn container_commands_require_wiring_and_star_agent_id() {
    let missing = block_on(execute_container_command(
        None,
        None,
        None,
        &serde_json::json!({"agent_id":"*","command":"get_containers"}),
    ));
    assert!(missing.is_error && missing.content.contains("not available"));

    let registry = EnvironmentRegistry::new();
    let uc = use_case(registry.clone());
    let query = Arc::new(ListEnvironmentsQuery::new(registry));
    let wrong_target = block_on(execute_container_command(
        Some(&query),
        Some(&uc),
        None,
        &serde_json::json!({"agent_id":"child","command":"get_containers"}),
    ));
    assert!(wrong_target.is_error && wrong_target.content.contains("agent_id '*'"));
}

#[test]
fn kill_container_decodes_exactly_one_string_target() {
    let uc = use_case(committed_registry());
    for args in [
        serde_json::json!({"agent_id":"*","command":"kill_container"}),
        serde_json::json!({"agent_id":"*","command":"kill_container","ref":"C1","name":"x"}),
        serde_json::json!({"agent_id":"*","command":"kill_container","ref":1}),
    ] {
        let result = block_on(execute_container_command(None, Some(&uc), None, &args));
        assert!(result.is_error, "{}", result.content);
    }
}

#[test]
fn listing_and_kill_round_trip_through_the_use_case() {
    let registry = committed_registry();
    let uc = use_case(registry.clone());
    let query = Arc::new(ListEnvironmentsQuery::new(registry));
    let listing = block_on(execute_container_command(
        Some(&query),
        Some(&uc),
        None,
        &serde_json::json!({"agent_id":"*","command":"get_containers"}),
    ));
    assert!(!listing.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&listing.content).unwrap();
    assert_eq!(parsed["containers"][0]["ref"], "C1");
    assert_eq!(parsed["containers"][0]["status"], "running");
    // Provenance (#2024 S4d): created here, by this (session-less) registry.
    assert_eq!(parsed["containers"][0]["restored"], false);
    assert_eq!(parsed["containers"][0]["session"], "");
    assert_eq!(parsed["containers"][0]["config"], "default");
    assert!(parsed["containers"][0]["created_at"].is_null());

    let killed = block_on(execute_container_command(
        None,
        Some(&uc),
        None,
        &serde_json::json!({"agent_id":"*","command":"kill_container","name":"tool-env"}),
    ));
    assert!(!killed.is_error, "{}", killed.content);
    let parsed: serde_json::Value = serde_json::from_str(&killed.content).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!({
            "killed":"C1",
            "agents":["impl-1517","rev-a"],
            "settled":[
                {"agent":"impl-1517","result":"graceful"},
                {"agent":"rev-a","result":"graceful"}
            ]
        })
    );

    let unknown = block_on(execute_container_command(
        None,
        Some(&uc),
        None,
        &serde_json::json!({"agent_id":"*","command":"kill_container","ref":"C9"}),
    ));
    assert!(unknown.is_error && unknown.content.contains("unknown"));
}

#[test]
fn kill_container_json_caps_long_member_lists() {
    let mut record = committed_registry().get("C1").unwrap();
    record.members = (1..=25).map(|n| format!("a{n}")).collect();
    let parsed = kill_container_result_json(
        &crate::application::environments::use_cases::KilledEnvironment {
            record,
            members: MemberShutdownReport::default(),
        },
    );
    assert_eq!(parsed["killed"], "C1");
    assert_eq!(parsed["agents"].as_array().unwrap().len(), 20);
    assert_eq!(parsed["omitted_agents"], 5);
}

#[test]
fn use_case_debug_is_redacted_but_present() {
    let uc = use_case(EnvironmentRegistry::new());
    assert!(format!("{uc:?}").contains("KillEnvironment"));
}

fn query_only_tool(query: Arc<ListEnvironmentsQuery>) -> super::super::agent_cmd::AgentCmdTool {
    super::super::agent_cmd::AgentCmdTool::new(super::super::subagent_registry::new_registry())
        .with_environment_control(super::EnvironmentControl {
            list: query,
            kill: use_case(EnvironmentRegistry::new()),
        })
}

#[test]
fn public_listing_query_only_empty_inventory_is_exact() {
    use crate::application::tools::ports::Tool;
    let query = Arc::new(ListEnvironmentsQuery::new(EnvironmentRegistry::new()));
    let result = block_on(
        query_only_tool(query.clone()).execute(r#"{"agent_id":"*","command":"get_containers"}"#),
    )
    .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(result.content, r#"{"containers":[]}"#);
    assert_eq!(query.execution_count(), 1);
}

#[test]
fn public_listing_query_only_preserves_complete_wire_objects_and_all_statuses() {
    use crate::application::tools::ports::Tool;
    let registry = EnvironmentRegistry::new();
    let mut expected = Vec::new();
    for (index, (status, label)) in [
        (EnvironmentStatus::Running, "running"),
        (EnvironmentStatus::Running, "empty"),
        (EnvironmentStatus::Killing, "killing"),
        (EnvironmentStatus::Stopped, "stopped"),
        (EnvironmentStatus::CleanupFailed, "cleanup-failed"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut record = committed_registry().get("C1").unwrap();
        record.environment_ref = registry.mint_ref();
        record.status = status;
        record.name = (index % 2 == 0).then(|| format!("name-{index}"));
        record.repository = format!("https://example.test/repo-{index}.git");
        record.workspace_path = format!("/workspace/with space/{index}").into();
        record.metadata = serde_json::json!({"index": index, "nested": {"ready": true}});
        record.last_error = (index == 4).then(|| "cleanup detail".to_string());
        if index == 1 {
            record.members.clear();
        }
        expected.push(serde_json::json!({
            "ref": format!("C{}", index + 1),
            "name": record.name,
            "status": label,
            "workspace": format!("/workspace/with space/{index}"),
            "repository": format!("https://example.test/repo-{index}.git"),
            "environment_uuid": record.environment_uuid,
            "members": if index == 1 { Vec::<String>::new() } else { vec!["impl-1517".into(), "rev-a".into()] },
            "metadata": {"index": index, "nested": {"ready": true}},
            "last_error": record.last_error,
            "restored": false,
            "session": "",
            "config": "default",
            "created_at": null,
        }));
        registry.commit(record);
    }
    let query = Arc::new(ListEnvironmentsQuery::new(registry));
    let result = block_on(
        query_only_tool(query.clone()).execute(r#"{"agent_id":"*","command":"get_containers"}"#),
    )
    .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&result.content).unwrap(),
        serde_json::json!({"containers": expected})
    );
    assert_eq!(query.execution_count(), 1);
}

#[test]
fn public_listing_rejection_matrix_never_calls_query() {
    use crate::application::tools::ports::Tool;
    let query = Arc::new(ListEnvironmentsQuery::new(committed_registry()));
    let tool = query_only_tool(query.clone());
    for args in [
        "not json",
        "null",
        "[]",
        "{}",
        r#"{"command":"get_containers"}"#,
        r#"{"agent_id":null,"command":"get_containers"}"#,
        r#"{"agent_id":1,"command":"get_containers"}"#,
        r#"{"agent_id":{},"command":"get_containers"}"#,
        r#"{"agent_id":[],"command":"get_containers"}"#,
        r#"{"agent_id":true,"command":"get_containers"}"#,
        r#"{"agent_id":"child","command":"get_containers"}"#,
        r#"{"agent_id":"","command":"get_containers"}"#,
        r#"{"agent_id":" *","command":"get_containers"}"#,
        r#"{"agent_id":"*"}"#,
        r#"{"agent_id":"*","command":null}"#,
        r#"{"agent_id":"*","command":1}"#,
        r#"{"agent_id":"*","command":true}"#,
        r#"{"agent_id":"*","command":[]}"#,
        r#"{"agent_id":"*","command":{}}"#,
        r#"{"agent_id":"*","command":"GET_CONTAINERS"}"#,
        r#"{"agent_id":"*","command":"get_containers "}"#,
        r#"{"agent_id":"*","command":"unsupported"}"#,
        r#"{"agent_id":"*","command":"kill_container","ref":"C1"}"#,
    ] {
        let result = block_on(tool.execute(args)).unwrap();
        assert!(result.is_error, "{args}: {}", result.content);
        assert_eq!(query.execution_count(), 0, "{args}");
    }
    let unwired =
        super::super::agent_cmd::AgentCmdTool::new(super::super::subagent_registry::new_registry());
    let result =
        block_on(unwired.execute(r#"{"agent_id":"*","command":"get_containers"}"#)).unwrap();
    assert!(result.is_error);
    assert_eq!(
        result.content,
        "agent_cmd error: environment listing is not available in this session"
    );
    assert_eq!(query.execution_count(), 0);
}

#[test]
fn get_containers_marks_restored_environments_with_their_creating_session() {
    let registry = EnvironmentRegistry::new();
    let mut record = committed_registry().get("C1").unwrap();
    record.created_by = "cli:earlier".into();
    record.created_at = Some(1_700_000_000);
    registry.restore(vec![record]);
    let query = Arc::new(ListEnvironmentsQuery::new(registry));
    let listing = block_on(execute_container_command(
        Some(&query),
        None,
        None,
        &serde_json::json!({"agent_id":"*","command":"get_containers"}),
    ));
    let parsed: serde_json::Value = serde_json::from_str(&listing.content).unwrap();
    let entry = &parsed["containers"][0];
    assert_eq!(entry["restored"], true);
    assert_eq!(entry["session"], "cli:earlier");
    assert_eq!(entry["created_at"], 1_700_000_000);
    assert_eq!(
        entry["status"], "empty",
        "members of another session are not listed"
    );
}

struct FixedRoster(crate::application::environments::ports::ContainerConfigRosterReport);

impl crate::application::environments::ports::ContainerConfigRoster for FixedRoster {
    fn roster(
        &self,
    ) -> Result<crate::application::environments::ports::ContainerConfigRosterReport, String> {
        Ok(self.0.clone())
    }

    fn revision(&self) -> String {
        "fixed".into()
    }
}

#[test]
fn get_container_configs_encodes_the_inventory_and_needs_wiring() {
    use crate::application::environments::dto::{ContainerConfigEntry, ContainerConfigLayer};
    use crate::application::environments::ports::ContainerConfigRosterReport;
    use crate::application::environments::use_cases::ListContainerConfigs;

    let args = serde_json::json!({"agent_id":"*","command":"get_container_configs"});
    let missing = block_on(execute_container_command(None, None, None, &args));
    assert!(
        missing.is_error
            && missing
                .content
                .contains("container config listing is not available in this session"),
        "{}",
        missing.content
    );

    let query = Arc::new(ListContainerConfigs::new(Arc::new(FixedRoster(
        ContainerConfigRosterReport {
            configs: vec![
                ContainerConfigEntry {
                    name: "alpha".into(),
                    default: false,
                    layer: ContainerConfigLayer::Global,
                    repository: None,
                    problem: None,
                    joinable: false,
                },
                ContainerConfigEntry {
                    name: "r".into(),
                    default: true,
                    layer: ContainerConfigLayer::Overlay,
                    repository: Some("https://example.test/r".into()),
                    problem: None,
                    joinable: false,
                },
            ],
            overlay_withheld: false,
            diagnostics: vec!["warning: legacy".into()],
        },
    ))));
    let listing = block_on(execute_container_command(None, None, Some(&query), &args));
    assert!(!listing.is_error, "{}", listing.content);
    let parsed: serde_json::Value = serde_json::from_str(&listing.content).unwrap();
    assert_eq!(
        parsed,
        serde_json::json!({
            "container_configs": [
                {"name":"r","default":true,"source":"overlay","repository":"https://example.test/r","problem":null,"joinable":false},
                {"name":"alpha","default":false,"source":"global","repository":null,"problem":null,"joinable":false}
            ],
            "overlay_withheld": false,
            "diagnostics": ["warning: legacy"]
        })
    );
    let wrong_target = block_on(execute_container_command(
        None,
        None,
        Some(&query),
        &serde_json::json!({"agent_id":"child","command":"get_container_configs"}),
    ));
    assert!(wrong_target.is_error && wrong_target.content.contains("agent_id '*'"));
}
