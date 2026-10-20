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

/// #2617 review: a step file or reference names a wrong-typed field and an
/// unknown key without quoting the file's value, and every key or
/// reference it does name is escaped.
#[test]
fn a_workflow_step_error_never_quotes_a_value_and_escapes_what_it_names() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("shared.json"), step_json("shared")).unwrap();
    std::fs::write(
        tmp.path().join("typed.json"),
        r#"{"key":"t","label":"T","phase":"plan","guidance":98765432123}"#,
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("odd.json"),
        r#"{"key":"o","label":"O","phase":"plan","x\u001b[31m":1}"#,
    )
    .unwrap();
    std::fs::write(tmp.path().join("broken.json"), "{ nope").unwrap();
    let cfg = tmp.path().join("config.json");
    for (steps, expected) in [
        (
            r#"{"ref":"shared","guidance":12345678995}"#,
            "shared: a field has the wrong type or value at guidance",
        ),
        (
            r#""typed""#,
            "typed.json: a field has the wrong type or value at guidance",
        ),
        (r#""odd""#, r"odd.json: unknown field `x\u{1b}[31m`"),
        (
            r#"{"ref":"shared","x\u001b[31m":1}"#,
            r"step reference: unknown field `x\u{1b}[31m`",
        ),
        (
            r#""broken""#,
            "broken.json: malformed JSON at line 1 column 3",
        ),
        (r#""gone\u001b[31m""#, r"gone\u{1b}[31m.json"),
    ] {
        std::fs::write(
            &cfg,
            format!(
                r#"{{"workflow":{{"templates":[{{"id":"tpl","label":"Tpl","description":"d","steps":[{steps}]}}]}}}}"#
            ),
        )
        .unwrap();
        let err = Config::load(cfg.to_str().unwrap()).unwrap_err().to_string();
        assert!(err.contains(expected), "{expected} not in: {err}");
        for quoted in ["12345678995", "98765432123", "\u{1b}"] {
            assert!(!err.contains(quoted), "{quoted:?} quoted in: {err}");
        }
    }
}
