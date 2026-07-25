use crate::{QuectoWorld, common};
use cucumber::{then, when};
use std::path::{Path, PathBuf};

const PHASE_0_DOCS: &[&str] = &[
    "docs/prd-harness-architecture-hardening.md",
    "docs/architecture-design-records/README.md",
    "docs/uds-protocol.md",
    "docs/harness-architecture-map.md",
    "docs/protocol-capability-matrix.md",
];

const PHASE_0_ADRS: &[&str] = &[
    "docs/architecture-design-records/adr-0012-explicit-agent-turn-state-machine.md",
    "docs/architecture-design-records/adr-0013-uds-command-family-router.md",
    "docs/architecture-design-records/adr-0014-context-management-is-a-first-class-application-subsystem.md",
    "docs/architecture-design-records/adr-0015-subagent-lifecycle-state-machine.md",
    "docs/architecture-design-records/adr-0016-typed-identifiers-for-protocol-and-session-boundaries.md",
    "docs/architecture-design-records/adr-0017-protocol-evolution-matrix.md",
    "docs/architecture-design-records/adr-0018-contributor-change-cookbooks.md",
    "docs/architecture-design-records/adr-0019-role-segregated-domain-ports.md",
];

const OBSOLETE_PLANNING_DOCS: &[&str] = &[
    "docs/quecto-mcp-prd.md",
    "docs/scenarios/architecture_contract_guardrails.md",
    "docs/scenarios/gpt54_1m_context.md",
    "docs/scenarios/gpt55_standard_default.md",
    "docs/scenarios/openai_oauth_gpt54_responses_cache.md",
    "docs/scenarios/resume_pruning_and_provider_resilience.md",
    "docs/scenarios/subagent_completion_notifications_passive.md",
    "docs/scenarios/subagent_notification_dedupe.md",
    "docs/scenarios/subagent_notification_provenance.md",
    "docs/scenarios/tui_context_window_display.md",
];

#[when(expr = "I read the repository file {string}")]
fn when_read_repository_file(world: &mut QuectoWorld, relative_path: String) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    match common::read_repository_file(base, &relative_path) {
        Ok(content) => {
            world.stdout = content;
            world.stderr.clear();
            world.exit_code = 0;
        }
        Err(error) => {
            world.stdout.clear();
            world.stderr = error;
            world.exit_code = 1;
        }
    }
}

#[when("I inspect obsolete repository planning artifact paths")]
fn when_inspect_obsolete_planning_artifact_paths(world: &mut QuectoWorld) {
    world.stdout = OBSOLETE_PLANNING_DOCS.join("\n");
    world.stderr.clear();
    world.exit_code = 0;
}

#[then("the obsolete planning documents should be absent")]
fn then_obsolete_planning_documents_absent(world: &mut QuectoWorld) {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let existing = world
        .stdout
        .lines()
        .filter(|path| repo.join(path).exists())
        .collect::<Vec<_>>();

    assert!(
        existing.is_empty(),
        "obsolete planning artifacts should be removed: {existing:?}"
    );
}

#[then(
    "the workflow docs should describe pure-move refactors as separate PRs before or after motivating behavior"
)]
fn then_workflow_docs_describe_pure_move_refactors(world: &mut QuectoWorld) {
    // The in-step assertion satisfies the BDD quality gate (Then steps must
    // assert); the shared helper carries the detailed content checks.
    assert!(
        world.stdout.contains("Pure-move refactors"),
        "workflow docs should include pure-move refactor guidance"
    );
    common::assert_pure_move_refactor_guidance(&world.stdout);
}

#[when("I inspect the Phase 0 hardening documentation links")]
fn when_inspect_phase_0_hardening_documentation_links(world: &mut QuectoWorld) {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut checked = Vec::new();
    let mut missing = Vec::new();

    for path in PHASE_0_DOCS.iter().chain(PHASE_0_ADRS.iter()) {
        let full_path = repo.join(path);
        if !full_path.exists() {
            missing.push(format!("missing document: {path}"));
            continue;
        }
        checked.push((*path).to_string());
    }

    for path in PHASE_0_DOCS {
        let full_path = repo.join(path);
        let Ok(content) = std::fs::read_to_string(&full_path) else {
            missing.push(format!("unreadable document: {path}"));
            continue;
        };
        let Some(parent) = full_path.parent() else {
            missing.push(format!("document has no parent: {path}"));
            continue;
        };
        for target in markdown_link_targets(&content) {
            if is_external_or_anchor(&target) {
                continue;
            }
            let resolved = normalize_link_target(parent, &target);
            checked.push(format!("{path} -> {target}"));
            if !resolved.exists() {
                missing.push(format!("{path} -> {target}"));
            }
        }
    }

    world.stdout = checked.join("\n");
    world.stderr = missing.join("\n");
    world.exit_code = if missing.is_empty() { 0 } else { 1 };
}

#[then("the harness architecture map should cover the Phase 0 hardening surfaces")]
fn then_architecture_map_covers_phase_0_surfaces(world: &mut QuectoWorld) {
    for heading in [
        "## Turn execution",
        "## Context management",
        "## UDS dispatch",
        "## Subagent lifecycle",
        "## Persistence and session recovery",
    ] {
        assert!(
            world.stdout.contains(heading),
            "architecture map missing {heading}"
        );
    }
}

#[then("the harness architecture map should record baseline hardening checks")]
fn then_architecture_map_records_baseline_checks(world: &mut QuectoWorld) {
    for required in [
        "## Baseline subsystem checks",
        "## Baseline longest files",
        "cargo test -p quecto-agentic-harness --test repo_docs",
        "tests/bdd/uds_steps.rs",
    ] {
        assert!(
            world.stdout.contains(required),
            "architecture map missing baseline record {required}"
        );
    }
}

#[then("the protocol capability matrix should include the baseline UDS capabilities")]
fn then_protocol_matrix_includes_baseline_capabilities(world: &mut QuectoWorld) {
    for capability in [
        "Length-prefixed JSON frames",
        "Bounded `agent_end` / `turn_end` message references",
        "`get_messages` newest bounded page",
        "Child-targeted history forwarding",
    ] {
        assert!(
            world.stdout.contains(capability),
            "protocol capability matrix missing {capability}"
        );
    }
}

#[then("the Phase 0 hardening documentation links should resolve")]
fn then_phase_0_hardening_documentation_links_resolve(world: &mut QuectoWorld) {
    assert_eq!(
        world.exit_code, 0,
        "Phase 0 documentation links should resolve; missing:\n{}\nchecked:\n{}",
        world.stderr, world.stdout
    );
    assert!(
        world
            .stdout
            .contains("docs/uds-protocol.md -> protocol-capability-matrix.md"),
        "UDS protocol docs should link the protocol matrix; checked:\n{}",
        world.stdout
    );
    assert!(
        world.stdout.contains(
            "docs/architecture-design-records/README.md -> ../protocol-capability-matrix.md"
        ),
        "ADR index should link the protocol matrix; checked:\n{}",
        world.stdout
    );
}

fn markdown_link_targets(content: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for candidate in content.split("](").skip(1) {
        if let Some(target) = candidate.split(')').next() {
            targets.push(target.split('#').next().unwrap_or(target).to_string());
        }
    }
    targets
}

fn is_external_or_anchor(target: &str) -> bool {
    target.is_empty()
        || target.starts_with('#')
        || target.starts_with("http://")
        || target.starts_with("https://")
        || target.starts_with("mailto:")
}

fn normalize_link_target(parent: &Path, target: &str) -> PathBuf {
    let mut path = PathBuf::from(parent);
    for component in Path::new(target).components() {
        match component {
            std::path::Component::ParentDir => {
                path.pop();
            }
            std::path::Component::CurDir => {}
            other => path.push(other.as_os_str()),
        }
    }
    path
}
