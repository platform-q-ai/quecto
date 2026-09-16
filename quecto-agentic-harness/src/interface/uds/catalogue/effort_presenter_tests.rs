use super::*;
use crate::domain::provider::EffortLevel::{High, Low, Medium};

#[test]
fn state_view_renders_api_strings_and_an_empty_vocabulary_as_is() {
    assert_eq!(
        EffortStateView::new(Some(High), &[Low, Medium, High]),
        EffortStateView {
            effort: Some("high".into()),
            effort_levels: vec!["low".into(), "medium".into(), "high".into()],
        }
    );
    assert_eq!(EffortStateView::new(None, &[]), EffortStateView::default());
}

#[test]
fn change_and_error_render_the_legacy_shapes() {
    assert_eq!(
        render_change(&EffortChangeOutcome {
            effective: Low,
            vocabulary: vec![Low],
        }),
        serde_json::json!({ "effort": "low" })
    );
    let error = EffortChangeError::Unsupported {
        requested: "max".into(),
        model: "xai/grok-4.5".into(),
        vocabulary: vec![Low, Medium, High],
    };
    assert_eq!(
        render_error(&error),
        "invalid effort level \"max\" for xai/grok-4.5; valid levels: low, medium, high"
    );
}
