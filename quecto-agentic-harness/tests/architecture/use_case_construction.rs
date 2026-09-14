//! Epic #1929 follow-up (#1666): composition is the only place that
//! constructs the concrete runtime graph. Infrastructure's inbound event
//! sources (reapers, socket monitors, cleanup workers) and the interface's
//! dispatch loops invoke application use cases through handles composition
//! injects; none of them constructs a use case or assembles a graph.

use std::collections::BTreeSet;
use std::path::Path;

use super::teardown_authority::{production_code, production_files, walk};

/// Every use case and ports bundle declared under an application
/// `use_cases` role folder, by name: a `pub struct` whose fields hold an
/// injected collaborator (an `Arc<…>` port or use case, a domain registry
/// handle, or a ports bundle). Request/response records declared beside them hold
/// values only and are boundary data, not graph nodes. The inventory is
/// exact: a new use case joins [`USE_CASES`] or the test fails.
fn use_case_struct_names() -> BTreeSet<String> {
    let mut files = Vec::new();
    walk(Path::new("src/application"), &mut files);
    let mut names = BTreeSet::new();
    for path in files.iter().filter(|path| {
        path.contains("/use_cases/") && !path.ends_with("_tests.rs") && !path.contains("fakes")
    }) {
        let code = production_code(path);
        let mut current: Option<String> = None;
        for (_, line) in &code {
            if let Some(rest) = line.trim_start().strip_prefix("pub struct ") {
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                current = (!name.is_empty() && rest.trim_end().ends_with('{')).then_some(name);
                continue;
            }
            let Some(name) = current.as_ref() else {
                continue;
            };
            if line.trim_start().starts_with('}') {
                current = None;
            } else if line.contains("Arc<") || line.contains("Registry") || line.contains("Ports,")
            {
                names.insert(name.clone());
            }
        }
    }
    let expected: BTreeSet<String> = USE_CASES.iter().map(|name| name.to_string()).collect();
    assert_eq!(
        names, expected,
        "the use-case inventory changed; update USE_CASES"
    );
    names
}

/// The exact use-case and ports-bundle inventory of `src/application`.
const USE_CASES: &[&str] = &[
    "CompensateFailedLaunch",
    "CompensateFailedLaunchPorts",
    "ExecuteHarnessShutdown",
    "ExecuteHarnessShutdownPorts",
    "FinalizeEnvironmentMember",
    "FindUseCase",
    "HarnessShutdownTransaction",
    "KillDelegatedAgent",
    "KillDelegatedAgentPorts",
    "KillEnvironment",
    "ListEnvironmentsQuery",
    "ObserveOwnedChildExit",
    "OwnerConclusionPorts",
    "PrepareHarnessShutdown",
    "SettleDelegatedChild",
    "SettleDelegatedChildPorts",
    "TerminateAllDelegatedAgents",
    "TerminateAllDelegatedAgentsPorts",
    "TerminateDelegatedAgent",
    "WebFetchUseCase",
];

/// `line` constructs `name`: a `name::new(` call at a word boundary, or —
/// for a ports bundle, which has public fields — a `name {` literal (a
/// type position such as `Arc<name>` or `&name` is not a construction; a
/// use case's fields are private, so only its constructor builds it).
fn constructs(line: &str, name: &str) -> bool {
    line.match_indices(name).any(|(index, _)| {
        let before = line[..index]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
        let after = &line[index + name.len()..];
        before
            && (after.starts_with("::new(") || (name.ends_with("Ports") && after.starts_with(" {")))
    })
}

fn constructions_in(layer: &str) -> Vec<String> {
    let names = use_case_struct_names();
    let mut found = Vec::new();
    for path in production_files()
        .into_iter()
        .filter(|path| path.starts_with(layer))
    {
        for (line_no, line) in production_code(&path) {
            for name in &names {
                if constructs(&line, name) {
                    found.push(format!(
                        "{path}:{line_no} constructs `{name}`: `{}`",
                        line.trim()
                    ));
                }
            }
        }
    }
    found
}

/// (a) No production file under `src/infrastructure` or `src/interface`
/// constructs an application use case, its ports bundle or its request
/// record; there is no allowlist. Adapters hold `Arc<UseCase>` handles (or
/// a builder alias composition fills) and invoke them.
#[test]
fn infrastructure_and_interface_never_construct_a_use_case() {
    let mut violations = constructions_in("src/infrastructure");
    violations.extend(constructions_in("src/interface"));
    assert!(
        violations.is_empty(),
        "use cases are constructed in composition only:\n{}",
        violations.join("\n")
    );
    // The literal path spelling is refused too, so a fully qualified call
    // cannot slip past the name scan.
    for layer in ["src/infrastructure", "src/interface"] {
        for path in production_files()
            .into_iter()
            .filter(|path| path.starts_with(layer))
        {
            for (line_no, line) in production_code(&path) {
                assert!(
                    !(line.contains("use_cases::") && line.contains("::new(")),
                    "{path}:{line_no} names `use_cases::…::new(`: `{}`",
                    line.trim()
                );
            }
        }
    }
}

/// (b) Composition is the only layer outside the application itself that
/// constructs a use case: the application may compose a sibling use case
/// from the ports it was built with, and composition builds the rest.
/// Nothing else in the crate (entry points, lib) does.
#[test]
fn composition_is_the_only_layer_that_constructs_use_cases() {
    let names = use_case_struct_names();
    let mut outside = Vec::new();
    let mut in_composition = 0usize;
    for path in production_files() {
        for (line_no, line) in production_code(&path) {
            for name in &names {
                if !constructs(&line, name) {
                    continue;
                }
                if path.starts_with("src/composition/") {
                    in_composition += 1;
                } else if !path.starts_with("src/application/") {
                    outside.push(format!("{path}:{line_no} constructs `{name}`"));
                }
            }
        }
    }
    assert!(outside.is_empty(), "{}", outside.join("\n"));
    assert!(
        in_composition >= 10,
        "composition builds the graph ({in_composition} constructions seen)"
    );
    // The teardown, lifecycle and environment graphs each have a home.
    for (file, builder) in [
        (
            "src/composition/subagent_teardown.rs",
            "pub fn build_teardown_graph",
        ),
        (
            "src/composition/subagent_termination.rs",
            "pub fn install_termination_owners",
        ),
        (
            "src/composition/subagent_lifecycle.rs",
            "pub fn build_lifecycle_use_cases",
        ),
        (
            "src/composition/environments.rs",
            "pub fn build_environment_control",
        ),
        (
            "src/composition/environments.rs",
            "pub fn build_member_finalizer",
        ),
    ] {
        assert!(
            production_code(file)
                .iter()
                .any(|(_, l)| l.contains(builder)),
            "{file} declares `{builder}`"
        );
    }
}

/// The interface declares the handles it needs as plain structs and a
/// builder alias, constructed by composition and passed down through
/// `CliComposition`; it never names composition and never assembles a
/// graph. The old graph module is gone without a re-export shim.
#[test]
fn interface_declares_handles_and_composition_fills_them() {
    assert!(
        !Path::new("src/interface/cli/uds_teardown_graph.rs").exists(),
        "the interface graph module was retired"
    );
    let handles = production_code("src/interface/cli/uds_teardown_handles.rs");
    for declared in [
        "pub struct TeardownLoopInputs {",
        "pub struct TeardownHandles {",
        "pub fleet: Arc<TerminateAllDelegatedAgents>,",
        "pub controller: Arc<SubagentTeardownController>,",
    ] {
        assert!(
            handles.iter().any(|(_, l)| l.contains(declared)),
            "uds_teardown_handles.rs declares `{declared}`"
        );
    }
    for forbidden in ["::new(", "crate::composition", "TeardownGraph"] {
        assert!(
            !handles.iter().any(|(_, l)| l.contains(forbidden)),
            "uds_teardown_handles.rs holds handles only, never `{forbidden}`"
        );
    }
    let cli = production_code("src/interface/cli/mod.rs");
    assert!(
        cli.iter()
            .any(|(_, l)| l.contains("pub type TeardownHandlesBuilder ="))
    );
    assert!(
        cli.iter()
            .any(|(_, l)| l.contains("pub teardown_graph: TeardownHandlesBuilder,")),
        "CliComposition carries the handles builder"
    );
    let composition = production_code("src/composition/subagent_teardown.rs");
    assert!(composition.iter().any(|(_, l)| {
        l.contains("pub fn build_teardown_graph(inputs: TeardownLoopInputs) -> TeardownHandles")
    }));
    // The per-loop handles are built by the composition-provided builder
    // the loop was handed, with the loop's own runtime inputs.
    let multi = production_code("src/interface/cli/uds_multi.rs");
    assert!(
        multi
            .iter()
            .any(|(_, l)| l.contains("build(super::uds_teardown_handles::TeardownLoopInputs {")),
        "the loop calls composition's builder with its runtime inputs"
    );
    // Infrastructure's inbound event sources hold handles or slots.
    let spawn = production_code("src/infrastructure/tools/spawn_lifecycle.rs");
    assert!(
        spawn
            .iter()
            .any(|(_, l)| l.contains("pub fn with_lifecycle_slot("))
    );
    let wiring = production_code("src/infrastructure/tools/subagent_teardown_wiring.rs");
    assert!(
        wiring
            .iter()
            .any(|(_, l)| l.contains("pub struct SubagentLifecycleSlot("))
    );
    assert!(
        !wiring
            .iter()
            .any(|(_, l)| l.contains("fn build_lifecycle_use_cases")),
        "the wiring module declares handles, composition builds them"
    );
    let cleanup = production_code("src/infrastructure/tools/subagent_cleanup.rs");
    assert!(cleanup.iter().any(|(_, l)| l.contains(
        "pub type MemberFinalizer = fn(EnvironmentRegistry) -> FinalizeEnvironmentMember;"
    )));
    let native = production_code("src/infrastructure/extensions/native.rs");
    assert!(
        native
            .iter()
            .any(|(_, l)| l.contains(".with_lifecycle_slot(termination_slots.lifecycle.clone())"))
    );
    assert!(native.iter().any(|(_, l)| {
        l.contains(".with_environment_control_slot(termination_slots.environments.clone())")
    }));
}
