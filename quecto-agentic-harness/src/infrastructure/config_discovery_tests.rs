//! Slice 2 (workflow-composable-templates PRD §3.2) tests: workflow template
//! directory discovery — `workflow.dir` precedence, filename-stem ids,
//! steps/ never scanned, directory-over-inline warning, fail-fast errors.

use super::*;

/// Inline `workflow.templates` config JSON with a single `test` template
/// carrying the given steps (mirrors the helper in `config_tests.rs`).
fn workflow_config_with_steps(steps: &str) -> String {
    format!(
        r#"{{"workflow":{{"templates":[{{"id":"test","label":"Test","description":"d","steps":[{steps}]}}]}}}}"#
    )
}

/// Minimal valid template FILE body (no `id`: the id is the filename stem).
fn template_file_json(label: &str) -> String {
    format!(
        r#"{{"label":"{label}","description":"d","steps":[{{"key":"one","label":"One","phase":"green","guidance":"g"}}]}}"#
    )
}

fn write_file(path: &std::path::Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// Load a `Config` from JSON written into `dir`.
fn config_in(dir: &std::path::Path, json: &str) -> Config {
    let path = dir.join("config.json");
    std::fs::write(&path, json).unwrap();
    Config::load(path.to_str().unwrap()).unwrap()
}

fn template_ids(discovery: &WorkflowTemplateDiscovery) -> Vec<&str> {
    discovery.templates.iter().map(|t| t.id.as_str()).collect()
}

#[test]
fn test_discover_uses_configured_workflow_dir_with_filename_stem_id() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        &dir.path().join("wf/speedy.json"),
        &template_file_json("Speedy"),
    );
    let config = config_in(dir.path(), r#"{"workflow":{"dir":"wf"}}"#);

    let discovery = discover_workflow_templates(&config, dir.path(), None)
        .expect("configured workflow.dir should be discovered");
    assert_eq!(template_ids(&discovery), ["speedy"]);
    assert_eq!(discovery.templates[0].label, "Speedy");
    assert!(
        discovery
            .source_dir
            .as_deref()
            .is_some_and(|d| d.ends_with("wf")),
        "source_dir should be the configured directory: {:?}",
        discovery.source_dir
    );
}

#[test]
fn test_discover_prefers_configured_dir_over_repo_local_dir() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        &dir.path().join("wf/configured.json"),
        &template_file_json("C"),
    );
    write_file(
        &dir.path().join(".quecto/workflows/repo_local.json"),
        &template_file_json("R"),
    );
    let config = config_in(dir.path(), r#"{"workflow":{"dir":"wf"}}"#);

    let discovery = discover_workflow_templates(&config, dir.path(), None).unwrap();
    assert_eq!(template_ids(&discovery), ["configured"]);
}

#[test]
fn test_discover_repo_local_quecto_workflows_when_dir_unset() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        &dir.path().join(".quecto/workflows/foo.json"),
        &template_file_json("Foo"),
    );
    let config = config_in(dir.path(), "{}");

    let discovery = discover_workflow_templates(&config, dir.path(), None)
        .expect("repo-local .quecto/workflows should be discovered");
    // AC3: dropping foo.json into the workflow dir makes template `foo`
    // appear with no config edit.
    assert_eq!(template_ids(&discovery), ["foo"]);
    assert!(
        discovery
            .source_dir
            .as_deref()
            .is_some_and(|d| d.ends_with(".quecto/workflows")),
        "source_dir should be the repo-local directory: {:?}",
        discovery.source_dir
    );
}

#[test]
fn test_discover_home_quecto_workflows_when_no_repo_local_dir() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    write_file(
        &home.join(".quecto/workflows/bar.json"),
        &template_file_json("Bar"),
    );
    let cwd = dir.path().join("repo");
    std::fs::create_dir_all(&cwd).unwrap();
    let config = config_in(&cwd, "{}");

    let discovery = discover_workflow_templates(&config, &cwd, Some(&home))
        .expect("~/.quecto/workflows should be discovered");
    assert_eq!(template_ids(&discovery), ["bar"]);
}

#[test]
fn test_discover_repo_local_dir_wins_over_home_dir() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    write_file(
        &home.join(".quecto/workflows/home_tpl.json"),
        &template_file_json("H"),
    );
    let cwd = dir.path().join("repo");
    write_file(
        &cwd.join(".quecto/workflows/repo_tpl.json"),
        &template_file_json("R"),
    );
    let config = config_in(&cwd, "{}");

    let discovery = discover_workflow_templates(&config, &cwd, Some(&home)).unwrap();
    assert_eq!(template_ids(&discovery), ["repo_tpl"]);
}

#[test]
fn test_discover_falls_back_to_inline_templates_without_warning() {
    // AC5: existing configs with inline workflow.templates keep working
    // identically when no workflow directory exists anywhere.
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(
        dir.path(),
        &workflow_config_with_steps(
            r#"{"key":"one","label":"One","phase":"green","guidance":"g"}"#,
        ),
    );

    let discovery = discover_workflow_templates(&config, dir.path(), None)
        .expect("inline templates must keep loading with no workflow dir");
    assert_eq!(template_ids(&discovery), ["test"]);
    assert_eq!(
        discovery.source_dir, None,
        "inline fallback has no source dir"
    );
    assert_eq!(discovery.warning, None, "inline fallback must not warn");
}

#[test]
fn test_discover_directory_wins_over_inline_templates_with_warning() {
    let dir = tempfile::tempdir().unwrap();
    write_file(
        &dir.path().join(".quecto/workflows/from_dir.json"),
        &template_file_json("D"),
    );
    let config = config_in(
        dir.path(),
        &workflow_config_with_steps(
            r#"{"key":"one","label":"One","phase":"green","guidance":"g"}"#,
        ),
    );

    let discovery = discover_workflow_templates(&config, dir.path(), None).unwrap();
    assert_eq!(
        template_ids(&discovery),
        ["from_dir"],
        "the directory wins; inline templates are ignored"
    );
    let warning = discovery
        .warning
        .expect("shadowed inline templates must produce a startup warning");
    assert!(
        warning.contains("inline") && warning.contains("ignored"),
        "warning should say the inline templates are ignored: {warning}"
    );
}

#[test]
fn test_discover_ignores_steps_subfolder_and_non_json_files() {
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".quecto/workflows");
    write_file(&wf.join("real.json"), &template_file_json("Real"));
    // Never templates: anything under steps/ (or any subfolder), non-.json.
    write_file(
        &wf.join("steps/reviews/shared.json"),
        r#"{"key":"shared","label":"Shared","phase":"review","guidance":"g"}"#,
    );
    write_file(&wf.join("nested/other.json"), &template_file_json("Nested"));
    write_file(&wf.join("README.md"), "not a template");
    let config = config_in(dir.path(), "{}");

    let discovery = discover_workflow_templates(&config, dir.path(), None).unwrap();
    assert_eq!(
        template_ids(&discovery),
        ["real"],
        "only top-level *.json files are templates"
    );
}

#[test]
fn test_discover_resolves_step_references_relative_to_workflow_dir() {
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".quecto/workflows");
    write_file(
        &wf.join("steps/reviews/shared.json"),
        r#"{"key":"shared","label":"Shared","phase":"review","guidance":"from the step file"}"#,
    );
    write_file(
        &wf.join("uses_ref.json"),
        r#"{"label":"U","description":"d","steps":["steps/reviews/shared"]}"#,
    );
    let config = config_in(dir.path(), "{}");

    let discovery = discover_workflow_templates(&config, dir.path(), None).unwrap();
    let step = &discovery.templates[0].steps[0];
    assert_eq!(step.key, "shared");
    assert_eq!(step.guidance.as_deref(), Some("from the step file"));
}

#[test]
fn test_discover_shared_step_edit_changes_every_referencing_template() {
    // AC2: a step file referenced by two templates is defined exactly once on
    // disk; editing it changes both templates on the next load.
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".quecto/workflows");
    write_file(
        &wf.join("steps/shared.json"),
        r#"{"key":"shared","label":"Shared","phase":"green","guidance":"v1"}"#,
    );
    for name in ["one.json", "two.json"] {
        write_file(
            &wf.join(name),
            r#"{"label":"T","description":"d","steps":["steps/shared"]}"#,
        );
    }
    let config = config_in(dir.path(), "{}");

    let first = discover_workflow_templates(&config, dir.path(), None).unwrap();
    for t in &first.templates {
        assert_eq!(
            t.steps[0].guidance.as_deref(),
            Some("v1"),
            "template {}",
            t.id
        );
    }

    write_file(
        &wf.join("steps/shared.json"),
        r#"{"key":"shared","label":"Shared","phase":"green","guidance":"v2"}"#,
    );
    let second = discover_workflow_templates(&config, dir.path(), None).unwrap();
    assert_eq!(second.templates.len(), 2);
    for t in &second.templates {
        assert_eq!(
            t.steps[0].guidance.as_deref(),
            Some("v2"),
            "editing the shared step file must change template {}",
            t.id
        );
    }
}

#[test]
fn test_discover_invalid_template_json_fails_naming_file() {
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".quecto/workflows");
    write_file(&wf.join("broken.json"), "not json {");
    let config = config_in(dir.path(), "{}");

    let err = discover_workflow_templates(&config, dir.path(), None)
        .expect_err("unparseable template JSON must fail startup")
        .to_string();
    assert!(
        err.contains("broken.json"),
        "error must name the file: {err}"
    );
}

#[test]
fn test_discover_unknown_template_field_fails_naming_file() {
    // PRD Decision 2: deny_unknown_fields — typos are load errors.
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".quecto/workflows");
    write_file(
        &wf.join("typo.json"),
        r#"{"labell":"T","description":"d","steps":[{"key":"one","label":"One","phase":"green"}]}"#,
    );
    let config = config_in(dir.path(), "{}");

    let err = discover_workflow_templates(&config, dir.path(), None)
        .expect_err("an unknown template field must fail startup")
        .to_string();
    assert!(err.contains("typo.json"), "error must name the file: {err}");
}

#[test]
fn test_discover_explicit_id_field_in_template_file_fails_naming_file() {
    // The template id IS the filename stem; an explicit `id` field would allow
    // two sources of truth to disagree, so it is a load error.
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".quecto/workflows");
    write_file(
        &wf.join("stem.json"),
        r#"{"id":"other","label":"T","description":"d","steps":[{"key":"one","label":"One","phase":"green"}]}"#,
    );
    let config = config_in(dir.path(), "{}");

    let err = discover_workflow_templates(&config, dir.path(), None)
        .expect_err("an explicit id field in a template file must fail startup")
        .to_string();
    assert!(err.contains("stem.json"), "error must name the file: {err}");
}

#[test]
fn test_discover_template_with_zero_steps_fails_naming_file() {
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".quecto/workflows");
    write_file(
        &wf.join("empty.json"),
        r#"{"label":"E","description":"d","steps":[]}"#,
    );
    let config = config_in(dir.path(), "{}");

    let err = discover_workflow_templates(&config, dir.path(), None)
        .expect_err("a template with zero steps must fail startup")
        .to_string();
    assert!(
        err.contains("empty.json"),
        "error must name the file: {err}"
    );
}

#[test]
fn test_discover_duplicate_resolved_step_keys_fail_naming_file_and_key() {
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".quecto/workflows");
    write_file(
        &wf.join("dups.json"),
        r#"{"label":"D","description":"d","steps":[
            {"key":"same","label":"A","phase":"green"},
            {"key":"same","label":"B","phase":"green"}
        ]}"#,
    );
    let config = config_in(dir.path(), "{}");

    let err = discover_workflow_templates(&config, dir.path(), None)
        .expect_err("duplicate resolved step keys must fail startup")
        .to_string();
    assert!(err.contains("dups.json"), "error must name the file: {err}");
    assert!(
        err.contains("same"),
        "error must name the duplicate key: {err}"
    );
}

#[test]
fn test_discover_missing_step_reference_fails_naming_file() {
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".quecto/workflows");
    write_file(
        &wf.join("dangling.json"),
        r#"{"label":"D","description":"d","steps":["steps/missing"]}"#,
    );
    let config = config_in(dir.path(), "{}");

    let err = discover_workflow_templates(&config, dir.path(), None)
        .expect_err("a dangling step reference must fail startup")
        .to_string();
    assert!(
        err.contains("steps/missing.json"),
        "error must name the referenced file: {err}"
    );
}

#[test]
fn test_discover_missing_configured_dir_fails_naming_path() {
    // An explicitly configured workflow.dir that does not exist is a hard
    // error — never a silent fall-through to another source.
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path(), r#"{"workflow":{"dir":"no-such-dir"}}"#);

    let err = discover_workflow_templates(&config, dir.path(), None)
        .expect_err("a missing configured workflow.dir must fail startup")
        .to_string();
    assert!(
        err.contains("no-such-dir"),
        "error must name the configured directory: {err}"
    );
}

#[test]
fn test_discover_rejects_too_many_template_files() {
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".quecto/workflows");
    for i in 0..=crate::domain::workflow::MAX_TEMPLATE_COUNT {
        write_file(&wf.join(format!("t{i}.json")), &template_file_json("T"));
    }
    let config = config_in(dir.path(), "{}");

    let err = discover_workflow_templates(&config, dir.path(), None)
        .expect_err("more than MAX_TEMPLATE_COUNT template files must fail startup")
        .to_string();
    assert!(
        err.contains("too many workflow templates"),
        "error should state the template-count bound: {err}"
    );
}

#[test]
fn test_load_workflow_templates_from_dir_loads_canonical_style_layout() {
    // The bare directory loader backs both discovery and the repo drift tests
    // that pin the canonical `workflows/` folder (AC7).
    let dir = tempfile::tempdir().unwrap();
    write_file(&dir.path().join("alpha.json"), &template_file_json("Alpha"));
    write_file(&dir.path().join("beta.json"), &template_file_json("Beta"));

    let mut templates = load_workflow_templates_from_dir(dir.path())
        .expect("a directory of template files should load");
    templates.sort_by(|a, b| a.id.cmp(&b.id));
    let ids: Vec<&str> = templates.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["alpha", "beta"]);
}

#[test]
fn test_discover_accepts_exactly_max_template_files() {
    // Companion boundary to `test_discover_rejects_too_many_template_files`:
    // exactly MAX_TEMPLATE_COUNT files must load (guards `>` vs `>=`).
    let dir = tempfile::tempdir().unwrap();
    let wf = dir.path().join(".quecto/workflows");
    for i in 0..crate::domain::workflow::MAX_TEMPLATE_COUNT {
        write_file(&wf.join(format!("t{i:02}.json")), &template_file_json("T"));
    }
    let config = config_in(dir.path(), "{}");

    let discovery = discover_workflow_templates(&config, dir.path(), None)
        .expect("exactly MAX_TEMPLATE_COUNT template files must load");
    assert_eq!(
        discovery.templates.len(),
        crate::domain::workflow::MAX_TEMPLATE_COUNT
    );
}

#[test]
fn test_discover_configured_dir_with_no_templates_fails_naming_dir() {
    // A resolved workflow directory is the single source of truth; a configured
    // dir that holds only a steps/ subfolder (no top-level *.json) must be a
    // hard error, never a silent fall-through to the engine's built-in
    // defaults.
    let dir = tempfile::tempdir().unwrap();
    write_file(
        &dir.path().join("wf/steps/shared.json"),
        r#"{"key":"shared","label":"Shared","phase":"green","guidance":"g"}"#,
    );
    let config = config_in(dir.path(), r#"{"workflow":{"dir":"wf"}}"#);

    let err = discover_workflow_templates(&config, dir.path(), None)
        .expect_err("a template-less configured workflow.dir must fail startup")
        .to_string();
    assert!(err.contains("wf"), "error must name the directory: {err}");
    assert!(
        err.contains("no templates"),
        "error must explain there are no templates: {err}"
    );
}

#[test]
fn test_discover_repo_local_dir_with_no_templates_fails_naming_dir() {
    // Same invariant for an auto-discovered repo-local directory: an existing
    // .quecto/workflows with no top-level *.json is an error, not a silent
    // fall-through to built-in defaults (and it does NOT cascade to ~/).
    let dir = tempfile::tempdir().unwrap();
    write_file(
        &dir.path().join(".quecto/workflows/steps/shared.json"),
        r#"{"key":"shared","label":"Shared","phase":"green","guidance":"g"}"#,
    );
    let config = config_in(dir.path(), "{}");

    let err = discover_workflow_templates(&config, dir.path(), None)
        .expect_err("a template-less repo-local workflow dir must fail startup")
        .to_string();
    assert!(
        err.contains(".quecto/workflows"),
        "error must name the directory: {err}"
    );
    assert!(
        err.contains("no templates"),
        "error must explain there are no templates: {err}"
    );
}

#[test]
fn test_discover_auto_discovered_dir_warns_even_without_inline_templates() {
    // An auto-discovered directory silently replaces the built-in default
    // templates; that switch must be surfaced as a startup warning even when no
    // inline templates are being shadowed.
    let dir = tempfile::tempdir().unwrap();
    write_file(
        &dir.path().join(".quecto/workflows/foo.json"),
        &template_file_json("Foo"),
    );
    let config = config_in(dir.path(), "{}");

    let discovery = discover_workflow_templates(&config, dir.path(), None).unwrap();
    let warning = discovery
        .warning
        .expect("an auto-discovered directory replacing the defaults must warn");
    assert!(
        warning.contains("discovered directory")
            && warning.contains("built-in default templates are not in use"),
        "warning should state the built-in defaults are not in use: {warning}"
    );
}

#[test]
fn test_discover_explicitly_configured_dir_does_not_warn_without_inline_templates() {
    // An explicitly configured workflow.dir is a deliberate user choice, not a
    // surprise, so it is silent when there are no inline templates to shadow.
    let dir = tempfile::tempdir().unwrap();
    write_file(
        &dir.path().join("wf/speedy.json"),
        &template_file_json("Speedy"),
    );
    let config = config_in(dir.path(), r#"{"workflow":{"dir":"wf"}}"#);

    let discovery = discover_workflow_templates(&config, dir.path(), None).unwrap();
    assert_eq!(
        discovery.warning, None,
        "an explicitly configured dir with no inline templates must not warn"
    );
}

#[test]
fn test_discover_empty_world_keeps_default_fallback_intact() {
    // Precedence terminus: no workflow.dir, no repo/home directory, no inline
    // templates. Discovery must succeed with an empty library and no warning,
    // preserving the engine's built-in `default_templates()` fallback.
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path(), "{}");

    let discovery = discover_workflow_templates(&config, dir.path(), None)
        .expect("an empty world must not fail discovery");
    assert!(
        discovery.templates.is_empty(),
        "no source anywhere yields an empty library (engine defaults apply)"
    );
    assert_eq!(discovery.source_dir, None);
    assert_eq!(discovery.warning, None);
}
