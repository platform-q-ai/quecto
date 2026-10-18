//! Structural bounds for cohesive serializer and application-driver extraction.
//! These filesystem checks make no claim about runtime behavior equivalence.

use std::fs;
use std::path::Path;

const SOURCE_CAPS: &[(&str, usize)] = &[
    ("src/infrastructure/providers/openai_request.rs", 200),
    (
        "src/infrastructure/providers/anthropic/request_body.rs",
        300,
    ),
    ("src/infrastructure/providers/codex_request.rs", 200),
    ("src/application/agent_turn/agent_loop/request.rs", 450),
    ("src/application/agent_turn/agent_loop/run_loop.rs", 350),
    ("src/infrastructure/providers/openai.rs", 750),
    ("src/infrastructure/providers/anthropic/mod.rs", 750),
    ("src/infrastructure/providers/codex.rs", 750),
    (
        "src/application/agent_turn/agent_loop/agent_loop_turn_flow.rs",
        750,
    ),
    ("src/application/agent_turn/agent_loop/mod.rs", 750),
];

fn source_violation(root: &Path, relative: &str, cap: usize) -> Option<String> {
    let path = root.join(relative);
    let diagnostic = |detail: String| format!("{}: {detail}; cap={cap}", path.display());
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => {
            return Some(diagnostic(
                "count=unavailable; require a real regular source file (no symlink)".into(),
            ));
        }
        Err(error) => {
            return Some(diagnostic(format!(
                "count=unavailable; require extracted/parent source file: {error}"
            )));
        }
    }

    match fs::read_to_string(&path) {
        Ok(source) => {
            let count = source.lines().count();
            match (source.split_whitespace().next().is_some(), count <= cap) {
                (true, true) => None,
                (false, _) => Some(diagnostic(format!(
                    "count={count}; require non-whitespace source content"
                ))),
                (true, false) => Some(diagnostic(format!(
                    "count={count}; extract cohesive responsibilities to meet the line bound"
                ))),
            }
        }
        Err(error) => Some(diagnostic(format!(
            "count=unavailable; require readable UTF-8 source: {error}"
        ))),
    }
}

#[test]
fn extracted_serializers_and_drivers_have_bounded_size() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let violations: Vec<String> = SOURCE_CAPS
        .iter()
        .filter_map(|&(relative, cap)| source_violation(root, relative, cap))
        .collect();

    assert!(
        violations.is_empty(),
        "serializer/driver extraction architecture violations (path, count, cap):\n{}",
        violations.join("\n")
    );
}
