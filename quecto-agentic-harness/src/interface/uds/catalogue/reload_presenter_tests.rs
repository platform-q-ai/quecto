use super::*;

#[test]
fn a_clean_reload_and_an_unchanged_one_are_a_bare_success() {
    assert_eq!(
        render(&ReloadOutcome::Reloaded {
            unknown_policy_tools: vec![],
        }),
        Ok(None)
    );
    assert_eq!(render(&ReloadOutcome::Unchanged), Ok(None));
}

/// #2247 round 2 N2: a reload that found `tools.policy` entries naming no
/// tool says so in its reply, each id with the warning a start-up prints.
#[test]
fn a_reload_with_unknown_policy_ids_carries_them_and_their_warnings() {
    let typo = "tool.v1:bundled-native:21:quecto:official-tools:bsah";
    assert_eq!(
        render(&ReloadOutcome::Reloaded {
            unknown_policy_tools: vec![typo.into(), "ghost".into()],
        }),
        Ok(Some(serde_json::json!({
            "unknownPolicyTools": [typo, "ghost"],
            "warnings": [
                format!("tools.policy: no tool has stable id '{typo}', so its entry never applies; fix or remove it under tools.policy.entries"),
                "tools.policy: no tool has stable id 'ghost', so its entry never applies; fix or remove it under tools.policy.entries",
            ],
        })))
    );
}

#[test]
fn a_failure_carries_the_rebuild_error_verbatim() {
    assert_eq!(
        render(&ReloadOutcome::Failed("expected value at line 1".into())),
        Err("expected value at line 1".to_string())
    );
}

#[test]
fn an_unconfigured_run_reports_the_legacy_message() {
    assert_eq!(
        render(&ReloadOutcome::NotConfigured),
        Err("provider reload is not configured".to_string())
    );
}
