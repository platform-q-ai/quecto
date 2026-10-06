//! Layout slice L3 (#2361): agent lifecycle modules live in `domain/agents`.
//! `external_agent` stays a sibling capability per the #2356 wiki decision
//! (wiki e61de22, Agentic-Harness-Target-Architecture "Target source tree":
//! "External member policy and use cases remain in `external_agent`").
use std::path::{Path, PathBuf};

const AGENT_MODULES: &[&str] = &[
    "agent",
    "subagent",
    "subagent_launch",
    "subagent_teardown",
    "parent_control",
    "harness_lifetime",
    "child_end",
    "child_session",
    "unread_report",
];
const EXTERNAL_AGENT_FILES: &[&str] = &["mod.rs", "backend.rs", "stream.rs", "turn.rs", "usage.rs"];

fn domain() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/domain")
}

#[test]
fn agent_modules_live_under_domain_agents() {
    let missing: Vec<_> = AGENT_MODULES
        .iter()
        .filter(|name| !domain().join("agents").join(format!("{name}.rs")).is_file())
        .collect();
    assert!(
        missing.is_empty(),
        "expected in src/domain/agents/: {missing:?}"
    );
}

#[test]
fn agent_modules_are_absent_from_domain_root() {
    let stray: Vec<_> = AGENT_MODULES
        .iter()
        .filter(|name| domain().join(format!("{name}.rs")).exists())
        .collect();
    assert!(stray.is_empty(), "still at src/domain root: {stray:?}");
}

#[test]
fn external_agent_stays_a_sibling_domain_capability() {
    let missing: Vec<_> = EXTERNAL_AGENT_FILES
        .iter()
        .filter(|file| !domain().join("external_agent").join(file).is_file())
        .collect();
    assert!(
        missing.is_empty(),
        "expected in src/domain/external_agent/: {missing:?}"
    );
    assert!(
        !domain().join("agents/external_agent").exists(),
        "external_agent must not nest under domain/agents (#2356 wiki e61de22)"
    );
}
