use std::sync::Arc;

use super::*;
use crate::application::catalogue::ports::EffortVocabularySource;

struct Fixed(Vec<EffortLevel>);

impl EffortVocabularySource for Fixed {
    fn effort_vocabulary(&self, _model: &str) -> Option<Vec<EffortLevel>> {
        Some(self.0.clone())
    }
}

fn control(levels: &[EffortLevel]) -> ChangeReasoningEffort {
    ChangeReasoningEffort::new(Arc::new(Fixed(levels.to_vec())))
}

fn config_with(effort: Option<&str>) -> Config {
    let mut config = Config::default();
    config.agents.defaults.effort = effort.map(str::to_string);
    config
}

#[test]
fn an_explicit_flag_the_model_accepts_is_applied() {
    let mut stderr = String::new();
    let admitted = admit(
        &control(&[EffortLevel::Low, EffortLevel::High]),
        Some(EffortLevel::High),
        &config_with(None),
        "xai/grok-4.5",
        &mut stderr,
    );
    assert_eq!(admitted, Some(Some(EffortLevel::High)));
    assert!(stderr.is_empty());
}

#[test]
fn an_explicit_flag_the_model_refuses_stops_startup_naming_the_levels() {
    let mut stderr = String::new();
    let admitted = admit(
        &control(&[EffortLevel::Low, EffortLevel::High]),
        Some(EffortLevel::XHigh),
        &config_with(None),
        "xai/grok-4.5",
        &mut stderr,
    );
    assert_eq!(admitted, None);
    assert!(stderr.contains("--effort"), "{stderr}");
    assert!(stderr.contains("valid levels: low, high"), "{stderr}");
}

#[test]
fn a_configured_default_is_admitted_or_dropped_with_a_warning() {
    let mut stderr = String::new();
    let kept = admit(
        &control(&[EffortLevel::Low, EffortLevel::High]),
        None,
        &config_with(Some("high")),
        "xai/grok-4.5",
        &mut stderr,
    );
    assert_eq!(kept, Some(Some(EffortLevel::High)));
    assert!(stderr.is_empty());

    let dropped = admit(
        &control(&[]),
        None,
        &config_with(Some("high")),
        "spark-local/qwen",
        &mut stderr,
    );
    assert_eq!(dropped, Some(None));
    assert!(
        stderr.contains("WARNING: configured effort 'high' is not accepted by spark-local/qwen"),
        "{stderr}"
    );
}

#[test]
fn no_flag_and_no_default_means_the_provider_default() {
    let mut stderr = String::new();
    assert_eq!(
        admit(
            &control(&[EffortLevel::Low]),
            None,
            &config_with(None),
            "m",
            &mut stderr
        ),
        Some(None)
    );
}
