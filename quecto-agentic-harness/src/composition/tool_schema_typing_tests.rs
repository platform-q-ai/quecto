//! Every built-in tool's parameters schema stays inside the subset each
//! provider adapter sends unchanged, with every property typed. The spawn
//! tool's `container` once carried only a description; models guessed its
//! type and sent a quoted JSON string, so every container spawn failed.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::application::search::ports::{
    PortFuture, Relevance, RelevanceCandidate, RelevanceJudge,
};
use crate::application::tools::ports::Tool;
use crate::domain::tool_policy::value_objects::tool::ToolDefinition;
use crate::domain::workflow::{WorkflowConfig, WorkflowEngine};
use crate::infrastructure::extensions::native::build_official_tool_registry;
use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::test_support::tool_schema_portability::portability_violations;
use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
use crate::infrastructure::tools::grep::GrepTool;
use crate::infrastructure::tools::panic_probe::PanicProbeTool;
use crate::infrastructure::tools::recall::RecallTool;
use crate::infrastructure::tools::spawn::SpawnTool;
use crate::infrastructure::tools::web_search::WebSearchTool;
use crate::infrastructure::tools::workflow_tool::WorkflowTool;

/// Every production `impl Tool for` type and the name its definition
/// carries among the checked definitions. `UdsExtensionTool` serves the
/// schema its extension declared, so it has no built-in schema to check.
const TOOL_TYPES: &[(&str, Option<&str>)] = &[
    ("ExecTool", Some("bash")),
    ("ReadTool", Some("read")),
    ("WriteTool", Some("write")),
    ("EditTool", Some("edit")),
    ("LsTool", Some("ls")),
    ("GrepTool", Some("grep")),
    ("FindTool", Some("find")),
    ("DocsTool", Some("docs")),
    ("SwarmTool", Some("swarm")),
    ("SpawnTool", Some("spawn")),
    ("AgentCmdTool", Some("agent_cmd")),
    (
        "KillDelegatedAgentTool",
        Some(crate::interface::tools::agent_cmd_kill::KILL_TOOL_NAME),
    ),
    ("WebSearchTool", Some("web_search")),
    ("WebFetchTool", Some("web_fetch")),
    (
        "WorkflowTool",
        Some(crate::infrastructure::tools::workflow_tool::WORKFLOW_TOOL_NAME),
    ),
    ("RecallTool", Some("recall")),
    (
        "PanicProbeTool",
        Some(crate::infrastructure::tools::panic_probe::PANIC_PROBE_TOOL),
    ),
    ("UdsExtensionTool", None),
];

/// The type a production source line implements `Tool` for, if it does.
fn tool_impl_type(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("impl")?;
    let (before, after) = rest.split_once("Tool for ")?;
    let path_ends_here = before.ends_with([' ', ':']);
    if !path_ends_here {
        return None;
    }
    let end = after
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(after.len());
    Some(&after[..end]).filter(|name| !name.is_empty())
}

/// Every `impl Tool for` type in the crate's production sources (test
/// modules, test directories and test support excluded).
fn production_tool_types(dir: &std::path::Path, found: &mut Vec<String>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.expect("directory entry").path())
        .collect();
    entries.sort();
    for path in entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if name != "tests" && name != "test_support" {
                production_tool_types(&path, found);
            }
        } else if name.ends_with(".rs") && !name.ends_with("_tests.rs") && name != "tests.rs" {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            found.extend(
                source
                    .lines()
                    .filter_map(tool_impl_type)
                    .map(str::to_string),
            );
        }
    }
}

#[test]
fn tool_impl_type_reads_impl_lines_only() {
    assert_eq!(
        tool_impl_type("impl Tool for SpawnTool {"),
        Some("SpawnTool")
    );
    assert_eq!(
        tool_impl_type("    impl crate::application::tools::ports::Tool for X {"),
        Some("X")
    );
    assert_eq!(
        tool_impl_type("// Shell execution tool: impl Tool for ExecTool (bash)."),
        None
    );
    assert_eq!(tool_impl_type("impl SubTool for Y {"), None);
    assert_eq!(tool_impl_type("impl Tool for {"), None);
    assert_eq!(
        tool_impl_type("impl<T: Send> Tool for Wrapper<T> {"),
        Some("Wrapper")
    );
    assert_eq!(tool_impl_type("implTool for Z {"), None);
}

#[test]
fn every_production_tool_type_is_checked_or_exempt() {
    let mut found = Vec::new();
    production_tool_types(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut found,
    );
    found.sort();
    let mut listed: Vec<String> = TOOL_TYPES.iter().map(|(t, _)| t.to_string()).collect();
    listed.sort();
    assert_eq!(
        found, listed,
        "list every production Tool in TOOL_TYPES and build it in built_in_definitions"
    );
}

/// A judge that never ranks: only its presence matters, since it adds
/// grep's `rank_by` property.
struct NoJudge;

impl RelevanceJudge for NoJudge {
    fn judge<'a>(
        &'a self,
        _query: &'a str,
        _candidates: &'a [RelevanceCandidate],
    ) -> PortFuture<'a, Relevance> {
        Box::pin(async { Relevance::Unavailable("not judged".into()) })
    }
}

/// The definitions of the built-in tools a harness can offer a model,
/// plus grep's ranked variant and the `agent_cmd kill` owner.
fn built_in_definitions(workspace: &std::path::Path) -> Vec<ToolDefinition> {
    let sandbox = Sandbox::new(Some(workspace.to_path_buf()));
    let registry = build_official_tool_registry(
        super::find::build_find_tool(Arc::new(workspace.to_path_buf()), Arc::new(sandbox.clone())),
        PathBuf::from(workspace),
        sandbox,
        Default::default(),
    );
    let mut definitions: Vec<ToolDefinition> = registry.definitions().to_vec();

    let workflow_engine = Arc::new(Mutex::new(
        WorkflowEngine::new(
            WorkflowConfig {
                auto_continue: true,
                completion_nudge: true,
                selector_prompt: None,
                dir: None,
                templates: vec![],
            },
            false,
        )
        .expect("default workflow templates are valid"),
    ));
    let recall = super::retention::retention_handles_over(Arc::new(FileContextSpillStore::new(
        FlatSessionLayout::new(workspace.join("spill")),
    )))
    .recall;
    let kill_tool =
        super::subagent_termination::build_kill_tool(crate::interface::cli::KillToolWiring {
            owner: crate::domain::ids::AgentUuid::new("harness".to_string()),
            registry: AgentCmdTool::new_registry(),
            broadcast_tx: None,
            notify_tx: None,
            harness_lifecycle:
                crate::infrastructure::tools::harness_lifecycle::new_shared_harness_lifecycle(),
            environment_registry: crate::domain::environments::entities::environment_registry::EnvironmentRegistry::new(),
            slots: Default::default(),
            base_dir: workspace.to_path_buf(),
        });
    let ranked_grep = GrepTool::new(
        Arc::new(workspace.to_path_buf()),
        Arc::new(Sandbox::new(Some(workspace.to_path_buf()))),
    )
    .with_relevance(Arc::new(NoJudge), 10);
    let others: Vec<Arc<dyn Tool>> = vec![
        Arc::new(SpawnTool::new(vec![])),
        Arc::new(AgentCmdTool::new(AgentCmdTool::new_registry())),
        kill_tool,
        Arc::new(ranked_grep),
        Arc::new(WebSearchTool::new(None)),
        Arc::new(WorkflowTool::new(workflow_engine)),
        Arc::new(RecallTool::new(recall, "session".to_string())),
        Arc::new(PanicProbeTool),
        super::web_fetch::build(Default::default(), 64),
    ];
    definitions.extend(others.iter().map(|tool| tool.definition()));
    definitions
}

#[test]
fn every_built_in_tool_schema_is_typed_and_portable() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let definitions = built_in_definitions(workspace.path());
    let names: Vec<&str> = definitions.iter().map(|d| d.name.as_ref()).collect();
    // Distinct names, so each listed type is the one supplying its name.
    let mut listed_names: Vec<&str> = TOOL_TYPES.iter().filter_map(|(_, name)| *name).collect();
    listed_names.sort_unstable();
    let count = listed_names.len();
    listed_names.dedup();
    assert_eq!(listed_names.len(), count, "TOOL_TYPES names repeat");
    for (tool_type, name) in TOOL_TYPES {
        if let Some(name) = name {
            assert!(
                names.contains(name),
                "{tool_type}'s {name} missing from {names:?}"
            );
        }
    }
    assert!(
        definitions
            .iter()
            .any(|d| d.name == "grep" && d.parameters_schema.contains("\"rank_by\"")),
        "the ranked grep schema is checked"
    );
    let failures: Vec<String> = definitions
        .iter()
        .flat_map(|definition| {
            let schema: serde_json::Value = serde_json::from_str(&definition.parameters_schema)
                .unwrap_or_else(|e| panic!("{}: schema is not JSON: {e}", definition.name));
            portability_violations(&schema)
                .into_iter()
                .map(move |violation| format!("{}: {violation}", definition.name))
        })
        .collect();
    assert_eq!(failures, Vec::<String>::new());
}
