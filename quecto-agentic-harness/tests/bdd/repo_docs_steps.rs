use crate::{QuectoWorld, common};
use cucumber::{then, when};
use std::path::Path;

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
