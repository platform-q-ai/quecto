//! Slice 2 (workflow-composable-templates PRD §3.2): directory discovery must
//! be WIRED into agent startup, not merely implemented as a helper. These
//! tests build the real tool registry (the startup path) and assert:
//! - a template dropped into `<cwd>/.quecto/workflows` is the library the
//!   session's workflow engine actually runs (AC3),
//! - a broken template file fails registry construction — startup — naming
//!   the file (AC4, fail fast at startup),
//! - the directory-shadows-inline warning is emitted on the startup stderr
//!   channel (the same surfacing path as other startup warnings),
//! - a bound `--workflow-spec` bypasses discovery entirely even when a
//!   workflow directory exists (AC6).
use super::*;
use crate::infrastructure::config::Config;

fn workflow_flags() -> AgentFlags {
    AgentFlags {
        session_name: None,
        no_session: false,
        message: None,
        system_prompt: None,
        model_override: None,
        max_iterations: None,
        max_time: None,
        uds_mode: true,
        no_sandbox: false,
        socket_path: None,
        persist: false,
        disabled_tools: vec![],
        effort: None,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: false,
        workflow_spec_path: None,
        inherited_tool_policy: None,
        parent_id: None,
        spawned: false,
        parent_identity_override: None,
        session_key_override: None,
    }
}

fn config_from_json(json: &str) -> Config {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), json).unwrap();
    Config::load(tmp.path().to_str().unwrap()).unwrap()
}

fn write_template(cwd: &std::path::Path, name: &str, label: &str) {
    write_template_json(
        cwd,
        name,
        &format!(
            r#"{{"label":"{label}","description":"d","steps":[{{"key":"one","label":"One","phase":"green","guidance":"g"}}]}}"#
        ),
    );
}

fn write_template_json(cwd: &std::path::Path, name: &str, json: &str) {
    let dir = cwd.join(".quecto/workflows");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), json).unwrap();
}

fn build(
    cwd: &std::path::Path,
    config: &Config,
    flags: &AgentFlags,
    stderr: &mut String,
) -> Result<ToolRegistryBuild, String> {
    let tmp = tempfile::TempDir::new().unwrap();
    build_tool_registry(ToolRegistryArgs {
        base_dir: tmp.path(),
        config_path: tmp.path(),
        config,
        http_client: &reqwest::Client::new(),
        flags,
        stderr,
        broadcast_tx: None,
        cwd,
        home_dir: None,
    })
}

fn engine_template_ids(build: &ToolRegistryBuild) -> Vec<String> {
    let engine = build
        .workflow_state
        .as_ref()
        .expect("workflow engine must be constructed at startup")
        .lock()
        .unwrap();
    engine
        .snapshot(true)
        .available_templates
        .iter()
        .map(|t| t.id.clone())
        .collect()
}

/// AC3 at the startup altitude: dropping `foo.json` into the workflow dir
/// makes template `foo` the library the session's engine runs — no config
/// edit, no helper-only shortcut.
#[test]
fn startup_engine_runs_templates_discovered_from_workflow_dir() {
    let cwd = tempfile::TempDir::new().unwrap();
    write_template(cwd.path(), "foo.json", "Foo");
    let config = config_from_json(r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);
    let mut stderr = String::new();
    let build = build(cwd.path(), &config, &workflow_flags(), &mut stderr)
        .expect("startup must succeed with a valid workflow dir");
    assert_eq!(engine_template_ids(&build), ["foo"]);
    // No inline templates to shadow, so there is no "inline ... ignored" warning.
    assert!(
        !stderr.contains("inline"),
        "no shadowing warning without inline templates: {stderr}"
    );
    // But an auto-discovered directory silently replaces the built-in default
    // templates, so that switch IS surfaced on startup stderr (never invisible).
    assert!(
        stderr.contains("WARNING")
            && stderr.contains("discovered directory")
            && stderr.contains("built-in default templates are not in use"),
        "an auto-discovered workflow dir must surface that it replaces the built-in defaults: {stderr}"
    );
}

/// AC4: a broken template file fails STARTUP (registry construction), naming
/// the offending file — never a session with a partial library.
#[test]
fn startup_fails_fast_when_a_workflow_dir_template_is_broken() {
    let cwd = tempfile::TempDir::new().unwrap();
    write_template_json(cwd.path(), "broken.json", "not json {");
    let config = config_from_json(r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);
    let mut stderr = String::new();
    let err = match build(cwd.path(), &config, &workflow_flags(), &mut stderr) {
        Err(err) => err,
        Ok(_) => panic!("a broken template file must fail startup"),
    };
    assert!(
        err.contains("broken.json"),
        "error must name the file: {err}"
    );
}

#[test]
fn startup_fails_fast_when_a_template_has_an_empty_step_key() {
    let cwd = tempfile::TempDir::new().unwrap();
    write_template_json(
        cwd.path(),
        "empty-key.json",
        r#"{"label":"Bad","description":"d","steps":[{"key":" ","label":"One","phase":"green"}]}"#,
    );
    let config = config_from_json(r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);
    let mut stderr = String::new();

    let err = match build(cwd.path(), &config, &workflow_flags(), &mut stderr) {
        Err(error) => error,
        Ok(_) => panic!("engine validation errors must abort startup"),
    };

    assert!(err.contains("failed to initialize workflow"), "{err}");
    assert!(err.contains("empty key"), "{err}");
}

#[test]
fn startup_fails_fast_when_a_guard_references_an_unknown_step() {
    let cwd = tempfile::TempDir::new().unwrap();
    write_template_json(
        cwd.path(),
        "bad-guard.json",
        r#"{"label":"Bad","description":"d","steps":[{"key":"one","label":"One","phase":"green"}],"guards":[{"commands":["git push"],"before_step_key":"missing","message":"blocked"}]}"#,
    );
    let config = config_from_json(r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);
    let mut stderr = String::new();

    let err = match build(cwd.path(), &config, &workflow_flags(), &mut stderr) {
        Err(error) => error,
        Ok(_) => panic!("engine validation errors must abort startup"),
    };

    assert!(err.contains("failed to initialize workflow"), "{err}");
    assert!(err.contains("unknown step key 'missing'"), "{err}");
}

/// The shadowing warning reaches the OBSERVABLE startup channel (stderr) —
/// the same path every other startup warning uses — not just a struct field.
#[test]
fn startup_surfaces_directory_shadows_inline_warning_on_stderr() {
    let cwd = tempfile::TempDir::new().unwrap();
    write_template(cwd.path(), "from_dir.json", "Dir");
    let config = config_from_json(
        r#"{"providers":{"openai":{"api_key":"sk-test"}},
            "workflow":{"templates":[{"id":"inline_tpl","label":"I","description":"d",
              "steps":[{"key":"one","label":"One","phase":"green"}]}]}}"#,
    );
    let mut stderr = String::new();
    let build = build(cwd.path(), &config, &workflow_flags(), &mut stderr).unwrap();
    assert_eq!(
        engine_template_ids(&build),
        ["from_dir"],
        "the directory must shadow the inline templates"
    );
    assert!(
        stderr.contains("WARNING") && stderr.contains("inline"),
        "the shadowing warning must be surfaced on startup stderr: {stderr}"
    );
}

/// AC6: a bound `--workflow-spec` session runs exactly the spec's template
/// even when a workflow directory exists on disk — discovery is bypassed, the
/// directory neither shadows the spec nor emits a spurious warning.
#[test]
fn bound_workflow_spec_bypasses_directory_discovery() {
    let cwd = tempfile::TempDir::new().unwrap();
    write_template(cwd.path(), "from_dir.json", "Dir");
    let spec = crate::domain::workflow::WorkflowSpec {
        template: crate::domain::workflow::WorkflowTemplate {
            id: "assigned".into(),
            label: "Assigned".into(),
            description: "d".into(),
            when_to_use: None,
            steps: vec![crate::domain::workflow::WorkflowTemplateStep {
                key: "one".into(),
                label: "One".into(),
                phase: "green".into(),
                guidance: None,
            }],
            guards: vec![],
        },
    };
    let spec_file = cwd.path().join("spec.json");
    std::fs::write(&spec_file, serde_json::to_string(&spec).unwrap()).unwrap();
    let config = config_from_json(r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);
    let mut flags = workflow_flags();
    flags.workflow_spec_path = Some(spec_file);
    let mut stderr = String::new();
    let build = build(cwd.path(), &config, &flags, &mut stderr)
        .expect("a bound spec session must start with a workflow dir present");
    assert_eq!(
        engine_template_ids(&build),
        ["assigned"],
        "the engine must run exactly the spec's template, not the directory's"
    );
    assert!(
        !stderr.contains("WARNING"),
        "a bound spec must not emit a discovery warning: {stderr}"
    );
    let engine = build.workflow_state.as_ref().unwrap().lock().unwrap();
    assert!(engine.is_bound(), "the spec engine must be bound");
}
