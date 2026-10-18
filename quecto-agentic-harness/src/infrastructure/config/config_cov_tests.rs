use super::*;
use tempfile::TempDir;

fn step_json(key: &str) -> String {
    format!(r#"{{"key":"{key}","label":"Label {key}","phase":"plan","guidance":"Do {key}"}}"#)
}

#[test]
fn workflow_step_string_ref_object_overrides_and_inline_ref_metadata() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("one.json"), step_json("one")).unwrap();
    std::fs::write(tmp.path().join("two.json"), step_json("two")).unwrap();
    let cfg = tmp.path().join("config.json");
    std::fs::write(
        &cfg,
        r#"{
          "workflow":{"templates":[{"id":"tpl","label":"Tpl","description":"d","steps":[
            "one",
            {"ref":"two","label":"Override"},
            {"key":"inline","label":"Inline","phase":"execute"}
          ]}]}
        }"#,
    )
    .unwrap();

    let loaded = Config::load(cfg.to_str().unwrap()).unwrap();
    let steps = &loaded.workflow.templates[0].steps;
    assert_eq!(steps[0].key, "one");
    assert_eq!(steps[1].key, "two");
    assert_eq!(steps[1].label, "Override");
    assert_eq!(steps[2].key, "inline");
}

#[test]
fn workflow_step_reference_rejects_bad_shapes_and_escape() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("config.json");
    std::fs::write(
        &cfg,
        r#"{"workflow":{"templates":[{"id":"tpl","label":"Tpl","description":"d","steps":[5]}]}}"#,
    )
    .unwrap();
    let err = Config::load(cfg.to_str().unwrap()).unwrap_err().to_string();
    assert!(err.contains("step entry must be"), "{err}");

    std::fs::write(
        &cfg,
        r#"{"workflow":{"templates":[{"id":"tpl","label":"Tpl","description":"d","steps":["../outside"]}]}}"#,
    )
    .unwrap();
    let err = Config::load(cfg.to_str().unwrap()).unwrap_err().to_string();
    assert!(err.contains("must remain within"), "{err}");
}

#[test]
fn workflow_step_file_validation_errors_name_the_reference() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("bad.json"),
        r#"{"key":"x","label":"X","phase":"plan","extra":1}"#,
    )
    .unwrap();
    let cfg = tmp.path().join("config.json");
    std::fs::write(
        &cfg,
        r#"{"workflow":{"templates":[{"id":"tpl","label":"Tpl","description":"d","steps":["bad"]}]}}"#,
    )
    .unwrap();
    let err = Config::load(cfg.to_str().unwrap()).unwrap_err().to_string();
    assert!(
        err.contains("bad.json") && err.contains("unknown field `extra`"),
        "{err}"
    );
}

#[test]
fn workflow_step_reference_object_rejects_non_string_ref() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("config.json");
    std::fs::write(
        &cfg,
        r#"{"workflow":{"templates":[{"id":"tpl","label":"Tpl","description":"d","steps":[{"ref":123,"label":"Override"}]}]}}"#,
    )
    .unwrap();

    let err = Config::load(cfg.to_str().unwrap()).unwrap_err().to_string();
    assert!(err.contains("`ref` must be a string"), "{err}");
}

#[test]
fn workflow_step_file_rejects_non_object_and_missing_required_fields() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("array.json"), "[]").unwrap();
    std::fs::write(
        tmp.path().join("missing_label.json"),
        r#"{"key":"x","phase":"plan"}"#,
    )
    .unwrap();
    let cfg = tmp.path().join("config.json");

    std::fs::write(
        &cfg,
        r#"{"workflow":{"templates":[{"id":"tpl","label":"Tpl","description":"d","steps":["array"]}]}}"#,
    )
    .unwrap();
    let err = Config::load(cfg.to_str().unwrap()).unwrap_err().to_string();
    assert!(err.contains("array.json"), "{err}");
    assert!(err.contains("expected a step object"), "{err}");

    std::fs::write(
        &cfg,
        r#"{"workflow":{"templates":[{"id":"tpl","label":"Tpl","description":"d","steps":["missing_label"]}]}}"#,
    )
    .unwrap();
    let err = Config::load(cfg.to_str().unwrap()).unwrap_err().to_string();
    assert!(err.contains("missing_label.json"), "{err}");
    assert!(err.contains("missing field"), "{err}");
}

#[test]
fn workflow_step_load_reports_nonexistent_base_dir() {
    let tmp = TempDir::new().unwrap();
    let missing_base = tmp.path().join("missing-base");

    let err = load_workflow_step(&missing_base, "step")
        .unwrap_err()
        .to_string();

    assert!(err.contains("missing-base"), "{err}");
}
