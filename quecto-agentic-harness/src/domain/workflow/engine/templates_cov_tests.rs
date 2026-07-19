use super::*;

#[test]
fn parse_embedded_template_accepts_reference_without_json_extension() {
    let template = parse_embedded_template(
        "ref-only",
        r#"{"label":"Ref Only","description":"desc","steps":["steps/shared/hook"]}"#,
        &[(
            "steps/shared/hook",
            r#"{"key":"hook","label":"Hook","phase":"red","guidance":"run hook"}"#,
        )],
    );

    assert_eq!(template.id, "ref-only");
    assert_eq!(template.steps[0].key, "hook");
    assert_eq!(template.steps[0].phase, "red");
}

#[test]
fn parse_embedded_template_preserves_inline_steps() {
    let template = parse_embedded_template(
        "inline",
        r#"{
            "label":"Inline",
            "description":"desc",
            "steps":[{"key":"write","label":"Write","phase":"green"}]
        }"#,
        &[],
    );

    assert_eq!(template.id, "inline");
    assert_eq!(template.steps.len(), 1);
    assert_eq!(template.steps[0].label, "Write");
    assert_eq!(template.steps[0].guidance, None);
}

#[test]
#[should_panic(expected = "must parse")]
fn parse_embedded_template_panics_on_invalid_json() {
    let _ = parse_embedded_template("bad-json", "not-json", &[]);
}

#[test]
#[should_panic(expected = "must be an object")]
fn parse_embedded_template_panics_on_non_object_json() {
    let _ = parse_embedded_template("array", "[]", &[]);
}

#[test]
#[should_panic(expected = "must have a steps array")]
fn parse_embedded_template_panics_without_steps_array() {
    let _ = parse_embedded_template(
        "no-steps",
        r#"{"label":"No steps","description":"desc"}"#,
        &[],
    );
}

#[test]
#[should_panic(expected = "references unembedded step")]
fn parse_embedded_template_panics_on_unknown_reference() {
    let _ = parse_embedded_template(
        "missing-ref",
        r#"{"label":"Missing","description":"desc","steps":["steps/missing"]}"#,
        &[],
    );
}
