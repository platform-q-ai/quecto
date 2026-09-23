//! Issue #2098: affirmative inventory of the zero-cost and live LLM capability
//! mirrors, with scenario-level tag ownership. Covers architecture.feature's
//! capability-checklist, manual-only and provider-smoke scenarios.
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const REQUIRED: &[(&str, &str, &str)] = &[
    (
        "text response",
        "token",
        "the mock LLM returns a text response",
    ),
    ("write", "write", "tool call for \"write\""),
    ("read", "read", "tool call for \"read\""),
    ("edit", "edit", "tool call for \"edit\""),
    ("bash", "shell", "tool call for \"bash\""),
    ("multi-step", "multi", "tool call sequence"),
    ("system prompt", "--system", "--system"),
    (
        "session memory",
        "session",
        "remembers context across session turns",
    ),
];

fn feature_files(dir: &Path, prefix: &str) -> Vec<(String, String)> {
    let mut files: Vec<_> = fs::read_dir(dir)
        .expect("feature directory exists")
        .map(|entry| entry.expect("read feature entry").path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".feature"))
        })
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let content = fs::read_to_string(path).expect("read feature content");
            (name, content)
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(!files.is_empty(), "missing feature inventory for {prefix}");
    files
}

fn tags(line: &str) -> BTreeSet<&str> {
    line.split_whitespace()
        .filter(|tag| tag.starts_with('@'))
        .collect()
}

fn scenario_tags(content: &str) -> Vec<BTreeSet<&str>> {
    let mut pending = BTreeSet::new();
    let mut feature_tags = BTreeSet::new();
    let mut scenarios = Vec::new();
    let mut in_examples = false;
    for line in content.lines().map(str::trim) {
        if line.starts_with('@') {
            if !in_examples {
                pending.extend(tags(line));
            }
        } else if line.starts_with("Feature:") {
            feature_tags = std::mem::take(&mut pending);
        } else if line.starts_with("Scenario:") || line.starts_with("Scenario Outline:") {
            in_examples = false;
            let mut effective = feature_tags.clone();
            effective.append(&mut pending);
            scenarios.push(effective);
        } else if line.starts_with("Examples:") {
            // Tags following Examples belong to that example table, never the next scenario.
            pending.clear();
            in_examples = true;
        } else if line.starts_with("Background:") || line.starts_with("Rule:") {
            pending.clear();
        }
    }
    assert!(!scenarios.is_empty(), "feature must contain scenarios");
    scenarios
}

fn validate_capabilities(real: &str, mock: &str) -> Result<(), String> {
    let real_lower = real.to_lowercase();
    for (name, real_marker, mock_marker) in REQUIRED {
        if !real_lower.contains(real_marker) {
            return Err(format!("live suite missing {name}"));
        }
        if !mock.contains(mock_marker) {
            return Err(format!("mock suite missing {name}"));
        }
    }
    Ok(())
}

fn validate_tags(
    files: &[(String, String)],
    required: &[&str],
    excluded: &[&str],
) -> Result<(), String> {
    for (name, content) in files {
        for (index, actual) in scenario_tags(content).iter().enumerate() {
            for tag in required {
                if !actual.contains(tag) {
                    return Err(format!("{name} scenario {} missing {tag}", index + 1));
                }
            }
            for tag in excluded {
                if actual.contains(tag) {
                    return Err(format!("{name} scenario {} includes {tag}", index + 1));
                }
            }
        }
    }
    Ok(())
}

#[test]
fn live_and_mock_capabilities_have_explicit_mirrors() {
    let real = feature_files(Path::new("tests/features"), "e2e_real_llm");
    let mock = feature_files(Path::new("tests/features"), "e2e_mock_llm");
    let joined = |files: &[(String, String)]| {
        files
            .iter()
            .map(|(_, body)| body.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    validate_capabilities(&joined(&real), &joined(&mock)).unwrap();
}

#[test]
fn every_mirror_and_smoke_scenario_has_its_own_lane() {
    let dir = Path::new("tests/features");
    validate_tags(
        &feature_files(dir, "e2e_mock_llm"),
        &["@mock-llm"],
        &["@real-llm", "@manual-real-llm"],
    )
    .unwrap();
    validate_tags(
        &feature_files(dir, "e2e_real_llm"),
        &["@manual-real-llm", "@mock-llm"],
        &["@real-llm"],
    )
    .unwrap();
    let smoke = fs::read_to_string(dir.join("provider_smoke.feature"))
        .expect("provider smoke feature exists");
    validate_tags(
        &[("provider_smoke.feature".into(), smoke)],
        &["@provider-smoke"],
        &["@mock-llm", "@manual-real-llm", "@real-llm"],
    )
    .unwrap();
}

#[test]
fn deliberate_capability_and_tag_violations_are_rejected() {
    assert!(validate_capabilities("token write read edit shell multi --system session", "the mock LLM returns a text response tool call for \"write\" tool call for \"read\" tool call for \"edit\" tool call for \"bash\" tool call sequence --system remembers context across session turns").is_ok());
    assert!(
        validate_capabilities(
            "token write read edit shell multi --system session",
            "the mock LLM returns a text response"
        )
        .is_err()
    );
    assert!(validate_capabilities("token", "the mock LLM returns a text response").is_err());
    let fixture = |body: &str| vec![("fixture.feature".to_string(), body.to_string())];
    assert!(
        validate_tags(
            &fixture("@mock-llm\nFeature: mirror\n  Scenario: one\n  Scenario: two"),
            &["@mock-llm"],
            &["@real-llm"]
        )
        .is_ok()
    );
    assert!(
        validate_tags(
            &fixture("Feature: mirror\n  @mock-llm\n  Scenario: one\n  Scenario: untagged"),
            &["@mock-llm"],
            &[]
        )
        .is_err()
    );
    assert!(
        validate_tags(
            &fixture("Feature: mirror\n  @mock-llm\n  Scenario Outline: first\n    Examples:\n      @mock-llm\n      | x |\n      | 1 |\n  Scenario: untagged"),
            &["@mock-llm"],
            &[]
        )
        .is_err(),
        "Examples tags must not leak into the following scenario"
    );
    assert!(
        validate_tags(
            &fixture("@provider-smoke\nFeature: smoke\n  @mock-llm\n  Scenario: leaked"),
            &["@provider-smoke"],
            &["@mock-llm"]
        )
        .is_err()
    );
}

#[test]
fn tag_tokens_ignore_surrounding_and_repeated_whitespace() {
    assert_eq!(
        tags(" \t@mock  @smoke\n "),
        BTreeSet::from(["@mock", "@smoke"])
    );
    assert!(tags(" \t\n ").is_empty());
}
