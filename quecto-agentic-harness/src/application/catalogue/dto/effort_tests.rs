use super::*;

#[test]
fn unsupported_names_the_model_and_its_vocabulary() {
    let error = EffortChangeError::Unsupported {
        requested: "xhigh".into(),
        model: "xai/grok-4.5".into(),
        vocabulary: vec![EffortLevel::Low, EffortLevel::Medium, EffortLevel::High],
    }
    .to_string();
    assert_eq!(
        error,
        "invalid effort level \"xhigh\" for xai/grok-4.5; valid levels: low, medium, high"
    );
}

#[test]
fn no_control_tells_the_user_how_to_declare_it() {
    let error = EffortChangeError::NoEffortControl {
        requested: "high".into(),
        model: "spark-local/qwen".into(),
    }
    .to_string();
    assert!(
        error.contains("spark-local/qwen has no reasoning-effort control"),
        "{error}"
    );
    assert!(error.contains("reasoning: true"), "{error}");
}
