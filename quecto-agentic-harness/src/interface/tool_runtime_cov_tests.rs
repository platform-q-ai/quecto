use super::*;

#[test]
fn entrypoint_and_profile_policy_helpers_cover_all_variants() {
    assert!(ToolEntrypoint::CliAgent.agent_control_default_enabled());
    assert!(ToolEntrypoint::UdsAgent.agent_control_default_enabled());
    assert!(!ToolEntrypoint::Repl.agent_control_default_enabled());

    assert!(ToolEntrypoint::CliAgent.web_default_enabled());
    assert!(ToolEntrypoint::UdsAgent.web_default_enabled());
    assert!(!ToolEntrypoint::Repl.web_default_enabled());

    assert!(!ToolEntrypoint::CliAgent.workflow_supported());
    assert!(ToolEntrypoint::UdsAgent.workflow_supported());
    assert!(!ToolEntrypoint::Repl.workflow_supported());

    assert_eq!(
        ToolRuntimeProfileContext::from_spawned(false),
        ToolRuntimeProfileContext::Parent
    );
    assert_eq!(
        ToolRuntimeProfileContext::from_spawned(true),
        ToolRuntimeProfileContext::Child
    );
    assert!(!ToolRuntimeProfileContext::Parent.is_child());
    assert!(ToolRuntimeProfileContext::Child.is_child());
    assert_eq!(
        ToolRuntimeProfileContext::Parent.profile_context(),
        crate::domain::tool::ToolProfileContext::Parent
    );
    assert_eq!(
        ToolRuntimeProfileContext::Child.profile_context(),
        crate::domain::tool::ToolProfileContext::Child
    );

    let repl = ToolRuntimePolicyState::for_entrypoint(ToolEntrypoint::Repl);
    assert_eq!(repl.entrypoint, ToolEntrypoint::Repl);
    assert!(!repl.agent_control_default_enabled);
    assert!(!repl.web_default_enabled);
    assert!(!repl.workflow_supported);
    assert!(repl.inherited_tool_policy.is_none());
}

#[test]
fn load_workflow_spec_rejects_oversized_specs_before_reading() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("oversized.json");
    std::fs::write(
        &path,
        vec![b' '; crate::domain::workflow::MAX_WORKFLOW_SPEC_BYTES + 1],
    )
    .expect("write oversized spec");
    let err = load_workflow_spec(&path).expect_err("oversized spec must fail");
    assert!(err.contains("workflow spec too large"), "{err}");
    assert!(
        path.exists(),
        "oversized spec is rejected before consumption"
    );
}

#[test]
fn load_workflow_spec_reports_io_and_json_errors_and_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let missing = tmp.path().join("missing.json");
    let missing_err = load_workflow_spec(&missing).unwrap_err();
    assert!(
        missing_err.contains("No such") || missing_err.contains("os error"),
        "{missing_err}"
    );

    let bad = tmp.path().join("bad.json");
    std::fs::write(&bad, "not-json").unwrap();
    let json_err = load_workflow_spec(&bad).unwrap_err();
    assert!(
        json_err.contains("expected") || json_err.contains("invalid"),
        "{json_err}"
    );

    let good = tmp.path().join("good.json");
    std::fs::write(
        &good,
        serde_json::json!({
            "template": {
                "id": "cov-template",
                "label": "Coverage Template",
                "description": "test workflow spec",
                "steps": [
                    {"key": "one", "label": "One", "phase": "Act"}
                ]
            }
        })
        .to_string(),
    )
    .unwrap();
    let spec = load_workflow_spec(&good).unwrap();
    assert_eq!(spec.template.id, "cov-template");
    assert_eq!(spec.template.steps.len(), 1);
}

#[test]
fn load_workflow_spec_reports_missing_unreadable_and_malformed_specs() {
    let dir = tempfile::tempdir().expect("tempdir");

    let missing = dir.path().join("absent.json");
    let err = load_workflow_spec(&missing).expect_err("an absent spec must fail");
    assert!(!err.is_empty(), "error message should not be empty");

    let as_dir = dir.path().join("spec-dir.json");
    std::fs::create_dir(&as_dir).expect("create dir");
    load_workflow_spec(&as_dir).expect_err("a directory is not a readable spec");

    let bad = dir.path().join("bad.json");
    std::fs::write(&bad, b"{ not a spec").expect("write malformed spec");
    let err = load_workflow_spec(&bad).expect_err("malformed JSON must fail");
    assert!(
        err.contains("expected") || err.contains("key") || err.contains("column"),
        "expected a serde parse message, got: {err}"
    );
}

#[test]
fn load_workflow_spec_consumes_the_file_on_success() {
    use crate::domain::workflow::{WorkflowSpec, WorkflowTemplate, WorkflowTemplateStep};

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("spec.json");
    let spec = WorkflowSpec {
        template: WorkflowTemplate {
            id: "t1".into(),
            label: "T1".into(),
            description: "d".into(),
            when_to_use: None,
            steps: vec![WorkflowTemplateStep {
                key: "s".into(),
                label: "S".into(),
                phase: "p".into(),
                guidance: None,
            }],
            guards: vec![],
        },
    };
    std::fs::write(&path, serde_json::to_string(&spec).unwrap()).expect("write spec");

    let loaded = load_workflow_spec(&path).expect("a well-formed spec loads");
    assert_eq!(loaded.template.id, "t1");
    assert!(!path.exists(), "spec file was not consumed after loading");
}

#[test]
fn canonical_parent_config_path_absolutizes_relative_paths_and_keeps_broken_ones() {
    // A resolvable relative path canonicalizes to an absolute one (PR #1401
    // review: container spawns require the parent-config fallback to be
    // absolute, but the parent may have been launched with a relative
    // `--config`). `.` always resolves, without touching the cwd.
    let canonical =
        canonical_parent_config_path(Some(std::path::PathBuf::from("."))).expect("some");
    assert!(canonical.is_absolute());
    assert_eq!(
        canonical,
        std::env::current_dir().unwrap().canonicalize().unwrap()
    );

    // A path that cannot be canonicalized is forwarded verbatim so the
    // spawn-time error names the real value.
    let missing = std::path::PathBuf::from("definitely/not/a/real/config.json");
    assert_eq!(
        canonical_parent_config_path(Some(missing.clone())),
        Some(missing)
    );
    assert_eq!(canonical_parent_config_path(None), None);
}
