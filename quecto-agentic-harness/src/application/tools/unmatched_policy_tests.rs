use super::*;
use crate::domain::tool_descriptor::ToolSource;

const TYPO: &str = "tool.v1:bundled-native:21:quecto:official-tools:bsah";
const WORKFLOW: &str = "tool.v1:bundled-native:15:quecto:workflow:workflow";
const PYTHON_LAB: &str = "tool.v1:bundled-native:21:quecto:official-tools:python_lab";
const EXTENSION: &str = "tool.v1:uds:12:uds:client-a:weather";

/// One pass sorts every unmatched id into the typos worth a warning and the
/// rest, each with why it is quiet; order is kept within each.
#[test]
fn the_split_keeps_only_typos_as_unknown() {
    let split = split_unmatched_policy_entries(vec![
        EXTENSION.to_string(),
        TYPO.to_string(),
        WORKFLOW.to_string(),
        "bash".to_string(),
        PYTHON_LAB.to_string(),
    ]);
    assert_eq!(split.unknown, vec![TYPO.to_string(), "bash".to_string()]);
    assert_eq!(
        split.quiet,
        vec![
            (
                EXTENSION.to_string(),
                UnmatchedPolicyEntry::AwaitingRegistration {
                    source: ToolSource::Uds
                }
            ),
            (WORKFLOW.to_string(), UnmatchedPolicyEntry::BundledElsewhere),
            (
                PYTHON_LAB.to_string(),
                UnmatchedPolicyEntry::Retired {
                    removed_by: "#1684"
                }
            ),
        ]
    );
}

#[test]
fn nothing_unmatched_splits_into_nothing() {
    assert_eq!(
        split_unmatched_policy_entries(Vec::new()),
        UnmatchedPolicySplit::default()
    );
}
