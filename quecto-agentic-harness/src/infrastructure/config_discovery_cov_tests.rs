use super::*;
use tempfile::TempDir;

fn write_template(dir: &std::path::Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).unwrap();
}

fn valid_template(label: &str) -> String {
    format!(
        r#"{{"label":"{label}","description":"desc","steps":[{{"key":"plan","label":"Plan","phase":"plan"}}]}}"#
    )
}

#[test]
fn discovery_prefers_configured_dir_and_warns_inline_ignored() {
    let tmp = TempDir::new().unwrap();
    let workflows = tmp.path().join("wf");
    std::fs::create_dir(&workflows).unwrap();
    write_template(&workflows, "custom.json", &valid_template("Custom"));
    let mut config = Config::default();
    config.workflow.dir = Some("wf".into());
    config.workflow.templates = vec![crate::domain::workflow::WorkflowTemplate {
        id: "inline".into(),
        label: "Inline".into(),
        description: "ignored".into(),
        when_to_use: None,
        steps: vec![crate::domain::workflow::WorkflowTemplateStep {
            key: "x".into(),
            label: "X".into(),
            phase: "plan".into(),
            guidance: None,
        }],
        guards: vec![],
    }];

    let found = discover_workflow_templates(&config, tmp.path(), None).unwrap();
    assert_eq!(found.templates[0].id, "custom");
    assert_eq!(found.source_dir.as_deref(), Some(workflows.as_path()));
    assert!(
        found
            .warning
            .unwrap()
            .contains("inline workflow.templates are ignored")
    );
}

#[test]
fn discovery_errors_for_empty_or_missing_directory_and_falls_back_inline() {
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.workflow.dir = Some("missing".into());
    let err = discover_workflow_templates(&config, tmp.path(), None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("workflow.dir is not a directory"), "{err}");

    let empty = tmp.path().join("empty");
    std::fs::create_dir(&empty).unwrap();
    config.workflow.dir = Some("empty".into());
    let err = discover_workflow_templates(&config, tmp.path(), None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("contains no templates"), "{err}");

    config.workflow.dir = None;
    let found = discover_workflow_templates(&config, tmp.path(), None).unwrap();
    assert!(found.templates.is_empty());
    assert!(found.source_dir.is_none());
}

#[test]
fn load_template_files_resolve_refs_and_reject_strict_errors() {
    let tmp = TempDir::new().unwrap();
    let steps_dir = tmp.path().join("steps");
    std::fs::create_dir(&steps_dir).unwrap();
    std::fs::write(
        steps_dir.join("step.json"),
        r#"{"key":"refstep","label":"Ref","phase":"execute"}"#,
    )
    .unwrap();
    write_template(
        tmp.path(),
        "b.json",
        r#"{"label":"B","description":"desc","steps":[{"ref":"step","guidance":"extra"}],"guards":[{"commands":["bash"],"before_step_key":"refstep","message":"stop"}]}"#,
    );
    write_template(tmp.path(), "ignored.txt", "not json");

    let templates = load_workflow_templates_from_dir(tmp.path()).unwrap();
    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0].id, "b");
    assert_eq!(templates[0].steps[0].key, "refstep");
    assert_eq!(templates[0].steps[0].guidance.as_deref(), Some("extra"));

    write_template(
        tmp.path(),
        "a.json",
        r#"{"id":"bad","label":"A","description":"d","steps":[]}"#,
    );
    let err = load_workflow_templates_from_dir(tmp.path())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("a.json") && err.contains("unknown field `id`"),
        "{err}"
    );
}

#[test]
fn load_template_rejects_nested_step_guard_and_duplicate_keys() {
    let tmp = TempDir::new().unwrap();
    write_template(
        tmp.path(),
        "dup.json",
        r#"{"label":"Dup","description":"d","steps":[{"key":"x","label":"X","phase":"plan"},{"key":"x","label":"X2","phase":"execute"}]}"#,
    );
    let err = load_workflow_templates_from_dir(tmp.path())
        .unwrap_err()
        .to_string();
    assert!(err.contains("duplicate step key `x`"), "{err}");

    std::fs::remove_file(tmp.path().join("dup.json")).unwrap();
    write_template(
        tmp.path(),
        "guard.json",
        r#"{"label":"G","description":"d","steps":[{"key":"x","label":"X","phase":"plan","typo":true}]}"#,
    );
    let err = load_workflow_templates_from_dir(tmp.path())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("step entry") && err.contains("unknown field `typo`"),
        "{err}"
    );
}
