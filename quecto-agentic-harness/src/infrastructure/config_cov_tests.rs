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
