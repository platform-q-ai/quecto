use super::*;

#[test]
fn reloaded_and_unchanged_are_a_bare_success() {
    assert_eq!(
        render(&ReloadOutcome::Reloaded {
            unknown_policy_tools: vec!["ghost".into()],
        }),
        Ok(())
    );
    assert_eq!(render(&ReloadOutcome::Unchanged), Ok(()));
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
