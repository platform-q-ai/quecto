use super::*;
use crate::domain::tool_descriptor::ToolSource;
use crate::domain::tool_id::stable_tool_id;

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

/// An id in the bundled-native namespace that no bundled tool has, current
/// or retired, is a typo; so is anything that is not a stable id at all.
#[test]
fn a_misspelt_bundled_id_or_a_non_stable_id_is_unknown() {
    for stable_id in [
        "tool.v1:bundled-native:21:quecto:official-tools:bsah",
        // A bundled name under another provider is not the bundled tool.
        "tool.v1:bundled-native:9:elsewhere:bash",
        // Not the stable-id grammar: a bare name, a legacy id, a wrong
        // length, an unknown namespace, nothing.
        "bash",
        "tool.name.v0:bash",
        "tool.v1:uds:10:uds:runtime:bash",
        "tool.v1:plugin:3:ext:bash",
        "tool.v1:bundled-native:021:quecto:official-tools:bash",
        "",
    ] {
        assert_eq!(
            classify_unmatched_policy_entry(stable_id),
            UnmatchedPolicyEntry::Unknown,
            "{stable_id}"
        );
    }
}

/// A UDS extension's or a runtime tool's id names a tool that registers
/// after start-up: its entry applies when it does, so it is no typo.
#[test]
fn an_extension_or_runtime_id_awaits_its_registration() {
    for (stable_id, source) in [
        ("tool.v1:uds:12:uds:client-a:weather", ToolSource::Uds),
        (
            "tool.v1:runtime:15:com.example.ext:fetch-page",
            ToolSource::Runtime,
        ),
    ] {
        assert_eq!(
            classify_unmatched_policy_entry(stable_id),
            UnmatchedPolicyEntry::AwaitingRegistration { source },
            "{stable_id}"
        );
    }
}

/// Every namespace the grammar defines is classified affirmatively: the
/// bundled one against the catalogue, every other one as awaiting its
/// registration. A new `ToolSource` must be placed here.
#[test]
fn every_namespace_is_classified() {
    for source in [
        ToolSource::BundledNative,
        ToolSource::Uds,
        ToolSource::Runtime,
    ] {
        let expected = match source {
            ToolSource::BundledNative => UnmatchedPolicyEntry::Unknown,
            ToolSource::Uds | ToolSource::Runtime => {
                UnmatchedPolicyEntry::AwaitingRegistration { source }
            }
        };
        assert_eq!(
            classify_unmatched_policy_entry(&stable_tool_id(source, "p", "never_built")),
            expected,
            "{source:?}"
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

/// The one warning line every entrypoint prints for a typo.
#[test]
fn the_warning_names_the_id_and_where_to_fix_it() {
    assert_eq!(
        unknown_policy_entry_warning("tool.v1:bundled-native:3:web:serch"),
        "tools.policy: no tool has stable id 'tool.v1:bundled-native:3:web:serch', so its entry never applies; fix or remove it under tools.policy.entries"
    );
}

/// #2247 review: an entry key is echoed bounded and escaped — a key-path
/// tail is whatever the user or a file supplied.
#[test]
fn an_entry_key_is_shown_bounded_on_a_char_boundary() {
    let huge = "a".repeat(100 * 1024);
    let shown = shown_entry_id(&huge);
    assert_eq!(shown, format!("{}…", "a".repeat(SHOWN_ENTRY_ID_MAX_BYTES)));
    // A multi-byte character straddling the bound is left out whole.
    let straddling = format!("{}é", "a".repeat(SHOWN_ENTRY_ID_MAX_BYTES - 1));
    let shown = shown_entry_id(&format!("{straddling}tail"));
    assert_eq!(
        shown,
        format!("{}…", "a".repeat(SHOWN_ENTRY_ID_MAX_BYTES - 1))
    );
    // A key that fits is shown whole, without the ellipsis.
    let id = "tool.v1:bundled-native:21:quecto:official-tools:bash";
    assert_eq!(shown_entry_id(id), id);
}

#[test]
fn an_entry_key_is_shown_with_controls_escaped() {
    assert_eq!(
        shown_entry_id("ba\nsh\x1b[31mred\u{202e}"),
        "ba\\nsh\\u{1b}[31mred\\u{202e}"
    );
    let warning = unknown_policy_entry_warning(&format!("x\n{}", "y".repeat(100 * 1024)));
    assert!(!warning.contains('\n'), "{warning}");
    assert!(warning.len() < 512, "{}", warning.len());
    assert!(warning.contains("'x\\nyyy"), "{warning}");
}
