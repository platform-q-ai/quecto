use super::*;
use crate::application::admission::dto::NegotiationPlan;

const VALID: &str = r#"{"directory":"/tmp/quecto-authority","groups":{"g":{"capacity":1,"reserve":0,"min_interval_ms":1,"queue_capacity":1,"queue_timeout_ms":1,"attempt_timeout_ms":1,"fallback_base_ms":1,"max_cooldown_ms":1}},"aliases":{"a":"g"},"bindings":{"openai":"a"}}"#;
const INVALID: &str = r#"{"directory":"relative","groups":{"g":{"capacity":1,"reserve":0,"min_interval_ms":1,"queue_capacity":1,"queue_timeout_ms":1,"attempt_timeout_ms":1,"fallback_base_ms":1,"max_cooldown_ms":1}},"aliases":{"a":"g"},"bindings":{"openai":"a"}}"#;

fn config(section: Option<&str>) -> Config {
    let mut config = Config::default();
    config.admission = section.map(|s| serde_json::from_str(s).unwrap());
    config
}

#[test]
fn a_root_validates_its_section_and_a_child_inherits_whatever_it_says() {
    let use_case = NegotiateAuthority::new();
    let context = Path::new("/tmp/child.ctx");
    // A root: a valid section is the authority to join; an invalid one stops
    // startup here, naming the error.
    assert_eq!(
        plan(&config(Some(VALID)), None, &use_case).unwrap(),
        NegotiationPlan::Root {
            directory: "/tmp/quecto-authority".into()
        }
    );
    let error = plan(&config(Some(INVALID)), None, &use_case).unwrap_err();
    assert!(error.contains("absolute"), "{error}");
    assert_eq!(
        plan(&config(None), None, &use_case).unwrap(),
        NegotiationPlan::Disabled
    );
    // A child inherits unconditionally: its own `--config` may carry no
    // section, a valid one, or an *invalid* one — none of that is its
    // business (the section is global-only and the parent already composed).
    for section in [None, Some(VALID), Some(INVALID)] {
        assert_eq!(
            plan(&config(section), Some(context), &use_case).unwrap(),
            NegotiationPlan::Child {
                context: context.to_path_buf()
            },
            "section {section:?}"
        );
    }
}
