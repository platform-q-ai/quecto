#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

fn repo_file(relative_path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative_path)
}

pub fn read_repo_file(relative_path: &str) -> String {
    fs::read_to_string(repo_file(relative_path))
        .unwrap_or_else(|e| panic!("failed to read {relative_path}: {e}"))
}

/// Issue #1004: the reviewer wave is restructured into narrow finder angles
/// with find -> verify -> single-post waves; Conformance-to-AC leaves the wave.
/// Shared by the native-config, examples-config and docs-guide tests so all
/// three copies are pinned to the identical token set and none can silently
/// drop an angle or wave semantic.
pub fn assert_reviewer_finder_waves(g: &str, source: &str) {
    for token in [
        "Wave 1",
        "Wave 2",
        "Wave 3",
        "hunk",                   // line-by-line hunk scan
        "Removed-behavior audit", // deleted-line invariant re-establishment
        "Cross-file tracer",      // callers/callees/consumers of changed symbols
        // Distinctive #1004 phrasings: the bare tokens "Security"/"Performance"
        // already appeared in the pre-#1004 guidance, so they carried no RED
        // evidence on their own.
        "path traversal", // Security angle detail
        "Performance/efficiency",
        "Reuse",
        "altitude",           // bandaid vs. mechanism, same-defect-class grep
        "Clean architecture", // layering, quirk placement, test-only paths, API surface
        "falsifiability",     // test falsifiability
        "REFUTE",
        "CONFIRMED",
        "PLAUSIBLE",
        "REFUTED",
    ] {
        assert!(
            g.contains(token),
            "{source} reviewers guidance should contain `{token}`, got: {g}"
        );
    }
    let lower = g.to_lowercase();
    // Wave 1 angle semantics.
    assert!(
        lower.contains("same-defect-class") || lower.contains("elsewhere in the codebase"),
        "{source}: reuse/altitude angle should hunt the defect class elsewhere in the codebase: {g}"
    );
    assert!(
        lower.contains("test-constructed") || lower.contains("constants"),
        "{source}: falsifiability angle should reject assertions on test-constructed state/constants: {g}"
    );
    assert!(
        lower.contains("reverted"),
        "{source}: falsifiability angle should reject tests that pass with the implementation reverted: {g}"
    );
    // Clean-architecture angle semantics (added after the PR #1036 review found
    // caller-side quirk state, cfg(test) production forks and consumer-less pub
    // API that no existing angle was hunting).
    assert!(
        lower.contains("caller-side state"),
        "{source}: clean-architecture angle should place preserved quirks inside the shared mechanism, not caller-side state: {g}"
    );
    assert!(
        lower.contains("only for tests"),
        "{source}: clean-architecture angle should flag production code paths that exist only for tests: {g}"
    );
    assert!(
        lower.contains("outside its own module"),
        "{source}: clean-architecture angle should require new public API to have a consumer outside its own module: {g}"
    );
    // Finders are forbidden GitHub writes.
    assert!(
        lower.contains("no github writes") || lower.contains("never post to github"),
        "{source}: finders must be forbidden GitHub writes: {g}"
    );
    // Wave 1 finding contract: file:line bound to the structured-finding format
    // (a bare `file:line` substring already matched the pre-#1004 guidance, so
    // it carries no RED evidence on its own).
    assert!(
        g.contains("file:line, a one-line summary"),
        "{source}: Wave 1 findings must be structured as file:line + one-line summary: {g}"
    );
    assert!(
        lower.contains("concrete failure scenario"),
        "{source}: Wave 1 findings must include a concrete failure scenario: {g}"
    );
    // Wave 2: adversarial verdicts quote the proving/disproving line.
    assert!(
        lower.contains("quote the proving") || lower.contains("quoted line"),
        "{source}: verdicts must quote the proving/disproving line: {g}"
    );
    // Wave 2 is skipped when Wave 1 returns nothing (a bare `skip` would match
    // unrelated wording such as "do not skip any finding").
    assert!(
        lower.contains("skip wave 2"),
        "{source}: Wave 2 should be skipped when Wave 1 returns no findings: {g}"
    );
    // Multi-finder convergence on one line is a severity signal.
    assert!(
        lower.contains("converge") && lower.contains("severity signal"),
        "{source}: multi-finder convergence should be a severity signal: {g}"
    );
    // Wave 3: exactly one submitted review posted by the master.
    assert!(
        lower.contains("one submitted") || lower.contains("single submitted"),
        "{source}: the master must post exactly one submitted review: {g}"
    );
    // Conformance-to-AC leaves the wave (standalone `conformance` step keeps it).
    assert!(
        !g.contains("Conformance-to-AC"),
        "{source}: Conformance-to-AC must be removed from the reviewer wave: {g}"
    );
}

pub fn read_repository_file(base: &Path, relative_path: &str) -> Result<String, String> {
    let base = base
        .canonicalize()
        .map_err(|e| format!("failed to canonicalize repo root {:?}: {}", base, e))?;
    let path = base.join(relative_path);
    let resolved = path
        .canonicalize()
        .map_err(|e| format!("failed to resolve {:?}: {}", path, e))?;
    if !resolved.starts_with(&base) {
        return Err(format!("path escapes repo root: {:?}", resolved));
    }
    fs::read_to_string(&resolved).map_err(|e| format!("failed to read {:?}: {}", resolved, e))
}

pub fn assert_pure_move_refactor_guidance(content: &str) {
    let paragraph = content
        .split("\n\n")
        .find(|paragraph| paragraph.contains("Pure-move refactors"))
        .expect("workflow docs should include pure-move refactor guidance");
    let lower = paragraph.to_lowercase();

    for token in ["file extractions", "renames", "byte-identical moves"] {
        assert!(
            lower.contains(token),
            "pure-move refactor guidance should include example `{token}`; paragraph was: {paragraph}"
        );
    }

    assert!(
        lower.contains("own pr") || lower.contains("separate pr"),
        "pure-move refactor guidance should require a separate PR; paragraph was: {paragraph}"
    );
    assert!(
        lower.contains("before or after") && lower.contains("behavioral change"),
        "pure-move refactor guidance should allow ordering before or after the motivating behavioral change; paragraph was: {paragraph}"
    );
}
