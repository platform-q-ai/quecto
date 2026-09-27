use super::*;

#[test]
fn a_bundled_tool_is_bundled_elsewhere() {
    for (provider_id, name) in BUNDLED_TOOLS {
        assert_eq!(
            classify_unmatched_policy_entry(&bundled_stable_id(provider_id, name)),
            UnmatchedPolicyEntry::BundledElsewhere,
            "{name}"
        );
    }
}

#[test]
fn python_lab_is_retired_by_1684() {
    assert_eq!(
        classify_unmatched_policy_entry(
            "tool.v1:bundled-native:21:quecto:official-tools:python_lab"
        ),
        UnmatchedPolicyEntry::Retired {
            removed_by: "#1684"
        }
    );
}

#[test]
fn a_misspelt_or_foreign_id_is_unknown() {
    for stable_id in [
        "tool.v1:bundled-native:21:quecto:official-tools:bsah",
        // A bundled name under another provider is not the bundled tool.
        "tool.v1:bundled-native:9:elsewhere:bash",
        "tool.v1:uds:10:uds:runtime:bash",
        "bash",
        "",
    ] {
        assert_eq!(
            classify_unmatched_policy_entry(stable_id),
            UnmatchedPolicyEntry::Unknown,
            "{stable_id}"
        );
    }
}

#[test]
fn no_tool_is_both_bundled_and_retired() {
    for (provider_id, name, _) in RETIRED_TOOLS {
        assert!(
            !BUNDLED_TOOLS.contains(&(provider_id, name)),
            "{provider_id}:{name} is listed as bundled and retired"
        );
    }
}
