//! Epic #1929 close (#1940), second half: the concrete graph lives in
//! composition, whole-crate dependency direction with an exact legacy
//! baseline, domain purity, capability-local contracted ports and a
//! parse/present-only interface for the teardown surface.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use super::teardown_authority::{production_code, production_files, walk};

/// The concrete teardown graph is built in composition only: the fleet,
/// the common shutdown, the selected-termination owners and the environment
/// kill are constructed nowhere else in production.
#[test]
fn teardown_graph_and_shutdown_delivery_are_composed_in_composition() {
    let constructors = [
        "TerminateAllDelegatedAgents::new(",
        "ExecuteHarnessShutdown::new(",
        "KillDelegatedAgent::new(",
        "TerminateDelegatedAgent::new(",
        "SettleDelegatedChild::new(",
        "KillEnvironment::new(",
        "SubagentTeardownController::new(",
    ];
    let allowed = [
        "src/composition/subagent_teardown.rs",
        "src/composition/subagent_termination.rs",
        // The fleet builds its per-child settlement from the ports it was
        // composed with; the environment kill is assembled by the native
        // extension build over composition's member-shutdown owner.
        "src/application/subagents/use_cases/terminate_all_delegated_agents.rs",
        "src/infrastructure/extensions/native.rs",
    ];
    for path in production_files() {
        for (line_no, line) in production_code(&path) {
            for constructor in constructors {
                if line.contains(constructor) {
                    assert!(
                        allowed.contains(&path.as_str()),
                        "{path}:{line_no} builds `{constructor}` outside composition"
                    );
                }
            }
        }
    }
    let teardown = production_code("src/composition/subagent_teardown.rs");
    assert!(
        teardown
            .iter()
            .any(|(_, l)| l.contains("pub fn build_teardown_graph"))
    );
    assert!(
        teardown
            .iter()
            .any(|(_, l)| l.contains("pub fn build_fleet_teardown"))
    );
    let termination = production_code("src/composition/subagent_termination.rs");
    assert!(
        termination
            .iter()
            .any(|(_, l)| l.contains("pub fn build_termination_owners"))
    );
    assert!(
        termination
            .iter()
            .any(|(_, l)| l.contains("pub fn install_termination_owners"))
    );
    // Signal delivery reaches the controller through the graph only.
    let shutdown = production_code("src/interface/cli/uds_shutdown.rs");
    assert!(
        shutdown
            .iter()
            .any(|(_, l)| l.contains("termination_signal("))
    );
    assert!(
        !shutdown.iter().any(|(_, l)| l.contains("libc::")),
        "the signal watcher delivers, it never signals"
    );
}

/// Whole-crate dependency direction: composition → interface → application
/// → domain, infrastructure inward only. The two legacy interface files that
/// still reach composition for the find tool are an exact, non-growing
/// baseline.
#[test]
fn whole_crate_dependency_direction_holds() {
    let forbidden: &[(&str, &[&str])] = &[
        (
            "src/domain",
            &[
                "crate::application",
                "crate::interface",
                "crate::infrastructure",
                "crate::composition",
            ],
        ),
        (
            "src/application",
            &[
                "crate::interface",
                "crate::infrastructure",
                "crate::composition",
            ],
        ),
        (
            "src/infrastructure",
            &["crate::interface", "crate::composition"],
        ),
        ("src/interface", &["crate::composition"]),
    ];
    let baseline: &[(&str, &str)] = &[
        (
            "src/interface/tool_runtime.rs",
            "crate::composition::find::build_find_tool",
        ),
        (
            "src/interface/shared.rs",
            "crate::composition::find::build_find_tool",
        ),
        // Pre-#1929 legacy: the agent_cmd roster presenter lives in the
        // interface protocol module (tracked by the #1666 clean-architecture
        // epic, not a teardown seam).
        (
            "src/infrastructure/tools/agent_cmd.rs",
            "crate::interface::cli::protocol::build_compact_subagent_roster",
        ),
    ];
    let mut seen_baseline = BTreeSet::new();
    let mut violations = Vec::new();
    for (layer, outward) in forbidden {
        let mut files = Vec::new();
        walk(Path::new(layer), &mut files);
        for path in files
            .iter()
            .filter(|p| !p.ends_with("_tests.rs") && !p.contains("/tests/"))
        {
            for (line_no, line) in production_code(path) {
                for dependency in *outward {
                    if !line.contains(dependency) {
                        continue;
                    }
                    if let Some(entry) = baseline
                        .iter()
                        .find(|(file, needle)| file == path && line.contains(needle))
                    {
                        seen_baseline.insert(*entry);
                        continue;
                    }
                    violations.push(format!(
                        "{path}:{line_no} depends outward on `{dependency}`: `{}`",
                        line.trim()
                    ));
                }
            }
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("\n"));
    assert_eq!(
        seen_baseline.len(),
        baseline.len(),
        "a baselined outward dependency disappeared; remove it from the baseline"
    );
}

/// Pure teardown domain: no trait carrying I/O, no async, no process, socket
/// or runtime types in the lifecycle/teardown domain modules. Whole-crate,
/// the legacy domain files that still hold ports or tokio types are an exact
/// baseline that may only shrink. #1940 moved the teardown-related ports out
/// (`SubagentLaunchPorts` → `application::subagent_launch`; `ProcessControl`,
/// `ProcessObservation`, `Clock`, `SwarmLifecycle` → `application::swarm`);
/// what remains is owned by **#1960** (part of the #1666 clean-architecture
/// epic), one file per row below, and nothing else may join.
/// Whether `line` declares `item` (`trait Foo` / `type Foo`) at any
/// visibility, matching the whole identifier.
fn declares(line: &str, item: &str) -> bool {
    let trimmed = line.trim_start();
    let body = trimmed
        .strip_prefix("pub(crate) ")
        .or_else(|| trimmed.strip_prefix("pub(super) "))
        .or_else(|| trimmed.strip_prefix("pub "))
        .unwrap_or(trimmed);
    body.strip_prefix(item)
        .is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_'))
}

/// (domain file, retired declaration, capability-local ports file it moved to)
const RETIRED_DOMAIN_PORTS: &[(&str, &str, &str)] = &[
    // #1940
    (
        "src/domain/subagent_launch.rs",
        "trait SubagentLaunchPorts",
        "src/application/subagent_launch.rs",
    ),
    (
        "src/domain/swarm.rs",
        "trait ProcessControl",
        "src/application/swarm.rs",
    ),
    (
        "src/domain/swarm.rs",
        "trait ProcessObservation",
        "src/application/swarm.rs",
    ),
    (
        "src/domain/swarm.rs",
        "trait Clock",
        "src/application/swarm.rs",
    ),
    (
        "src/domain/swarm.rs",
        "trait SwarmLifecycle",
        "src/application/swarm.rs",
    ),
    // #1960
    (
        "src/domain/agent.rs",
        "trait AgentLoop",
        "src/application/agent_turn/ports.rs",
    ),
    (
        "src/domain/audit.rs",
        "trait AuditSink",
        "src/application/audit/ports.rs",
    ),
    ("src/domain/tool.rs", "trait Tool", TOOLS_PORTS),
    ("src/domain/tool.rs", "trait ToolGuard", TOOLS_PORTS),
    ("src/domain/tool.rs", "trait ToolCatalog", TOOLS_PORTS),
    ("src/domain/tool.rs", "trait ToolExecutor", TOOLS_PORTS),
    ("src/domain/tool.rs", "trait ToolPolicyMutator", TOOLS_PORTS),
    (
        "src/domain/tool.rs",
        "trait RuntimeToolLifecycleRegistry",
        TOOLS_PORTS,
    ),
    ("src/domain/tool.rs", "trait SessionAwareTools", TOOLS_PORTS),
    ("src/domain/tool.rs", "trait ToolRegistry", TOOLS_PORTS),
    (
        "src/domain/tool.rs",
        "trait ToolExecutionAdmission",
        TOOLS_PORTS,
    ),
    (
        "src/domain/extension.rs",
        "trait Extension",
        "src/application/extensions/ports.rs",
    ),
];

const TOOLS_PORTS: &str = "src/application/tools/ports.rs";

#[test]
fn domain_is_pure_and_the_legacy_baseline_does_not_grow() {
    let impure = [
        "tokio::",
        "std::process",
        "std::net",
        "unix::net",
        "libc::",
        "async fn",
        "Pin<Box<dyn Future",
    ];
    // (file, what keeps it here — tracked by #1960)
    let legacy_baseline: BTreeSet<&str> = [
        "src/domain/extension_tool.rs",      // #1960: tokio oneshot reply
        "src/domain/provider.rs",            // #1960: LlmProvider, RequestAdmission
        "src/domain/request_observation.rs", // #1960: RequestAccounting
        "src/domain/session.rs",             // #1960: SessionStore, ContextSpillStore
        "src/domain/subagent_launch.rs",     // #1960: LaunchFuture alias
        "src/domain/swarm.rs",               // #1960: CoordinationPort, SwarmRunControl
    ]
    .into_iter()
    .collect();
    // The ports #1940 and #1960 moved out must not come back, and each one
    // is declared in its capability-local ports file and nowhere else.
    for (file, retired_port, new_home) in RETIRED_DOMAIN_PORTS {
        // A domain file whose only content was the port is deleted outright.
        assert!(
            !Path::new(file).exists()
                || !production_code(file)
                    .iter()
                    .any(|(_, l)| declares(l, retired_port)),
            "{file} re-declares `{retired_port}`, which was moved to the application"
        );
        let declared_in: Vec<String> = production_files()
            .into_iter()
            .filter(|path| {
                production_code(path)
                    .iter()
                    .any(|(_, l)| declares(l, retired_port))
            })
            .collect();
        assert_eq!(
            declared_in,
            vec![new_home.to_string()],
            "`{retired_port}` must be declared in {new_home} and nowhere else"
        );
    }
    let mut files = Vec::new();
    walk(Path::new("src/domain"), &mut files);
    let mut legacy_seen = BTreeSet::new();
    for path in files.iter().filter(|p| !p.ends_with("_tests.rs")) {
        let code = production_code(path);
        let has_trait = code
            .iter()
            .any(|(_, l)| l.trim_start().starts_with("pub trait "));
        let has_impure = code
            .iter()
            .any(|(_, l)| impure.iter().any(|needle| l.contains(needle)));
        if has_trait || has_impure {
            assert!(
                legacy_baseline.contains(path.as_str()),
                "{path} adds a trait or an I/O/async/process type to the domain (trait: {has_trait}, impure: {has_impure})"
            );
            legacy_seen.insert(path.as_str());
        }
    }
    assert_eq!(
        legacy_seen,
        legacy_baseline.iter().copied().collect(),
        "a legacy domain file became pure: shrink the baseline"
    );
    for pure in [
        "src/domain/subagent_teardown.rs",
        "src/domain/parent_control.rs",
        "src/domain/harness_lifetime.rs",
        "src/domain/environment_retention.rs",
        "src/domain/environment_registry.rs",
    ] {
        assert!(!legacy_baseline.contains(pure), "{pure} is pure by design");
        assert!(Path::new(pure).exists(), "{pure} exists");
    }
}

/// Ports are capability-local: every `pub trait` of the teardown
/// capabilities lives in that capability's `ports.rs`, and every one of
/// them is proven by a contract suite in `tests/contracts/`.
#[test]
fn teardown_ports_are_capability_local_and_contracted() {
    let contracts = fs::read_to_string("tests/contracts.rs").expect("tests/contracts.rs");
    for capability in ["subagents", "environments"] {
        let mut files = Vec::new();
        walk(
            Path::new(&format!("src/application/{capability}")),
            &mut files,
        );
        for path in files.iter().filter(|p| !p.ends_with("_tests.rs")) {
            let traits: Vec<String> = production_code(path)
                .into_iter()
                .filter_map(|(_, line)| {
                    line.trim_start().strip_prefix("pub trait ").map(|rest| {
                        rest.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                            .next()
                            .unwrap_or_default()
                            .to_string()
                    })
                })
                .collect();
            if traits.is_empty() {
                continue;
            }
            assert!(
                path.ends_with(&format!("src/application/{capability}/ports.rs")),
                "{path} declares ports {traits:?} outside the capability's ports.rs"
            );
            for port in traits {
                let module = to_snake_case(&port);
                assert!(
                    contracts.contains(&format!("contracts/{module}.rs")),
                    "port {port} has no contract suite tests/contracts/{module}.rs"
                );
            }
        }
    }
}

fn to_snake_case(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Interface adapters parse, map, invoke and present: no interface module
/// of the teardown surface spawns, signals, or names a process or child type.
#[test]
fn teardown_interface_only_parses_maps_and_presents() {
    let forbidden = [
        "tokio::process",
        "std::process::Command",
        "libc::",
        "OwnedChildSupervisor::new",
        "request_termination(",
        ".kill(",
        "start_kill",
        "cascade_remove(",
        "cleanup_removed_entries",
        "mark_entry_dead(",
        "claim_stopping(",
        "claim_terminal(",
    ];
    let mut files = Vec::new();
    walk(Path::new("src/interface/uds/subagent_teardown"), &mut files);
    files.extend(
        [
            "src/interface/tools/agent_cmd_kill.rs",
            "src/interface/cli/uds_delete_all_subagents.rs",
            "src/interface/cli/uds_busy_subagents.rs",
            "src/interface/cli/uds_shutdown.rs",
            "src/interface/cli/uds_dispatch_session.rs",
            "src/interface/cli/uds_parent_control.rs",
            "src/interface/cli/uds_teardown_graph.rs",
        ]
        .into_iter()
        .map(str::to_string),
    );
    for path in files.iter().filter(|p| !p.ends_with("_tests.rs")) {
        for (line_no, line) in production_code(path) {
            for needle in forbidden {
                assert!(
                    !line.contains(needle),
                    "{path}:{line_no} does more than parse/map/present: `{needle}`"
                );
            }
        }
    }
}
