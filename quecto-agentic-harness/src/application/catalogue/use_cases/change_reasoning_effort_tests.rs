//! `ChangeReasoningEffort` against a fake vocabulary source and runtime.

use std::collections::HashMap;
use std::sync::Mutex;

use super::*;
use crate::application::catalogue::ports::{DefaultScope, RecordedDefaults};
use crate::domain::provider::EffortLevel::{High, Low, Max, Medium, None as NoneLevel, XHigh};

struct FakeVocabulary(HashMap<&'static str, Vec<EffortLevel>>);

impl EffortVocabularySource for FakeVocabulary {
    fn effort_vocabulary(&self, model: &str) -> Option<Vec<EffortLevel>> {
        self.0.get(model).cloned()
    }
}

fn use_case() -> ChangeReasoningEffort {
    use_case_persisting(Arc::new(RecordedDefaults::default()))
}

fn use_case_persisting(persistence: Arc<RecordedDefaults>) -> ChangeReasoningEffort {
    ChangeReasoningEffort::new(
        Arc::new(FakeVocabulary(HashMap::from([
            ("xai/grok-4.5", vec![Low, Medium, High]),
            ("xai/grok-4.6", vec![Low, Medium, High, XHigh]),
            (
                "openai-api/gpt-5.6",
                vec![NoneLevel, Low, Medium, High, XHigh],
            ),
            ("anthropic-api/opus", vec![Low, Medium, High, Max]),
            ("spark-local/qwen", vec![]),
        ]))),
        persistence,
    )
}

#[derive(Default)]
struct FakeRuntime {
    effort: Option<EffortLevel>,
    startup: Option<EffortLevel>,
    writes: Mutex<usize>,
}

impl EffortRuntime for FakeRuntime {
    fn effort(&self) -> Option<EffortLevel> {
        self.effort
    }
    fn startup_effort(&self) -> Option<EffortLevel> {
        self.startup
    }
    fn apply_effort(&mut self, level: Option<EffortLevel>) {
        *self.writes.lock().unwrap() += 1;
        self.effort = level;
    }
}

#[test]
fn choices_are_the_catalogue_vocabulary_or_empty_for_unknown_models() {
    let use_case = use_case();
    assert_eq!(use_case.choices("xai/grok-4.5"), vec![Low, Medium, High]);
    assert!(use_case.choices("spark-local/qwen").is_empty());
    assert!(use_case.choices("openrouter/mystery").is_empty());
}

#[test]
fn validate_accepts_only_the_models_own_levels() {
    let use_case = use_case();
    assert_eq!(use_case.validate("xai/grok-4.6", "xhigh"), Ok(XHigh));
    assert_eq!(
        use_case.validate("xai/grok-4.5", "xhigh"),
        Err(EffortChangeError::Unsupported {
            requested: "xhigh".into(),
            model: "xai/grok-4.5".into(),
            vocabulary: vec![Low, Medium, High],
        })
    );
    assert_eq!(
        use_case.validate("xai/grok-4.6", "none"),
        Err(EffortChangeError::Unsupported {
            requested: "none".into(),
            model: "xai/grok-4.6".into(),
            vocabulary: vec![Low, Medium, High, XHigh],
        })
    );
    assert!(matches!(
        use_case.validate("anthropic-api/opus", "turbo"),
        Err(EffortChangeError::Unsupported { .. })
    ));
}

#[test]
fn validate_distinguishes_no_control_from_an_unknown_model() {
    let use_case = use_case();
    assert_eq!(
        use_case.validate("spark-local/qwen", "low"),
        Err(EffortChangeError::NoEffortControl {
            requested: "low".into(),
            model: "spark-local/qwen".into(),
        })
    );
    assert_eq!(
        use_case.validate("openrouter/mystery", "low"),
        Err(EffortChangeError::UnknownModel {
            requested: "low".into(),
            model: "openrouter/mystery".into(),
        })
    );
}

#[test]
fn a_model_switch_resets_to_low_only_where_low_is_accepted() {
    let use_case = use_case();
    let mut runtime = FakeRuntime {
        effort: Some(XHigh),
        ..Default::default()
    };
    assert!(use_case.reset_for_model_switch(&mut runtime, "xai/grok-4.5"));
    assert_eq!(runtime.effort, Some(Low));
    assert!(
        !use_case.reset_for_model_switch(&mut runtime, "anthropic-api/opus"),
        "already low"
    );
    assert!(use_case.reset_for_model_switch(&mut runtime, "spark-local/qwen"));
    assert_eq!(runtime.effort, None, "no control: provider default");
}

#[test]
fn a_session_switch_restores_the_startup_default_as_admitted_for_the_active_model() {
    let use_case = use_case();
    // Started on gpt-5.6 with --effort xhigh, switched to grok-4.5, overrode
    // to medium, then opened a fresh session: xhigh is not grok-4.5's.
    let mut runtime = FakeRuntime {
        effort: Some(Medium),
        startup: Some(XHigh),
        ..Default::default()
    };
    assert!(use_case.restore_startup_default(&mut runtime, "xai/grok-4.5"));
    assert_eq!(
        runtime.effort, None,
        "the startup level is not admitted here"
    );
    assert!(use_case.restore_startup_default(&mut runtime, "openai-api/gpt-5.6"));
    assert_eq!(runtime.effort, Some(XHigh));
    assert!(!use_case.restore_startup_default(&mut runtime, "openai-api/gpt-5.6"));
}

#[test]
fn admit_keeps_a_level_only_when_the_model_accepts_it() {
    let use_case = use_case();
    assert_eq!(use_case.admit("xai/grok-4.6", Some(XHigh)), Some(XHigh));
    assert_eq!(use_case.admit("xai/grok-4.5", Some(XHigh)), None);
    assert_eq!(
        use_case.admit("openai-api/gpt-5.6", Some(NoneLevel)),
        Some(NoneLevel)
    );
    assert_eq!(use_case.admit("spark-local/qwen", Some(Low)), None);
    assert_eq!(use_case.admit("xai/grok-4.6", None), None);
}

#[test]
fn execute_applies_an_accepted_level_and_reports_the_vocabulary() {
    let use_case = use_case();
    let mut runtime = FakeRuntime::default();
    let outcome = use_case
        .execute(
            &mut runtime,
            &EffortChangeRequest {
                model: "xai/grok-4.5".into(),
                level: "high".into(),
                persist: None,
            },
        )
        .unwrap();
    assert_eq!(
        outcome,
        EffortChangeOutcome {
            effective: High,
            vocabulary: vec![Low, Medium, High],
            persisted: None,
        }
    );
    assert_eq!(runtime.effort, Some(High));
    assert_eq!(*runtime.writes.lock().unwrap(), 1);
}

#[test]
fn execute_is_a_no_op_write_when_the_level_is_already_in_effect() {
    let use_case = use_case();
    let mut runtime = FakeRuntime {
        effort: Some(High),
        ..Default::default()
    };
    use_case
        .execute(
            &mut runtime,
            &EffortChangeRequest {
                model: "xai/grok-4.5".into(),
                level: "high".into(),
                persist: None,
            },
        )
        .unwrap();
    assert_eq!(*runtime.writes.lock().unwrap(), 0);
}

#[test]
fn execute_leaves_the_runtime_untouched_on_refusal() {
    let use_case = use_case();
    let mut runtime = FakeRuntime {
        effort: Some(Medium),
        ..Default::default()
    };
    let error = use_case
        .execute(
            &mut runtime,
            &EffortChangeRequest {
                model: "xai/grok-4.5".into(),
                level: "xhigh".into(),
                persist: None,
            },
        )
        .unwrap_err();
    assert!(matches!(error, EffortChangeError::Unsupported { .. }));
    assert_eq!(runtime.effort, Some(Medium));
    assert_eq!(*runtime.writes.lock().unwrap(), 0);
}

#[test]
fn debug_does_not_expose_the_port() {
    assert_eq!(format!("{:?}", use_case()), "ChangeReasoningEffort { .. }");
}

// ── Persisting a default (#2024 S2) ─────────────────────────────────────────

#[test]
fn execute_with_persist_records_the_validated_level_then_applies_it() {
    let persistence = Arc::new(RecordedDefaults::default());
    let use_case = use_case_persisting(persistence.clone());
    let mut runtime = FakeRuntime::default();
    let outcome = use_case
        .execute(
            &mut runtime,
            &EffortChangeRequest {
                model: "xai/grok-4.5".into(),
                level: "high".into(),
                persist: Some(DefaultScope::Local),
            },
        )
        .unwrap();
    assert_eq!(
        persistence.records.lock().unwrap().as_slice(),
        &[(
            DefaultScope::Local,
            "agents.defaults.effort".to_string(),
            "high".to_string()
        )]
    );
    let persisted = outcome.persisted.expect("recorded");
    assert_eq!(persisted.scope, DefaultScope::Local);
    assert_eq!(runtime.effort, Some(High));
}

#[test]
fn a_refused_record_leaves_the_session_untouched_and_names_the_reason() {
    let persistence = Arc::new(RecordedDefaults::refusing("overlay is not trusted"));
    let use_case = use_case_persisting(persistence);
    let mut runtime = FakeRuntime {
        effort: Some(Low),
        ..Default::default()
    };
    let error = use_case
        .execute(
            &mut runtime,
            &EffortChangeRequest {
                model: "xai/grok-4.5".into(),
                level: "high".into(),
                persist: Some(DefaultScope::Global),
            },
        )
        .unwrap_err();
    assert_eq!(
        error,
        EffortChangeError::Persist {
            level: High,
            scope: DefaultScope::Global,
            reason: "overlay is not trusted".into(),
        }
    );
    assert!(error.to_string().contains("overlay is not trusted"));
    assert!(error.to_string().contains("global default"));
    assert_eq!(runtime.effort, Some(Low), "nothing applied");
    assert_eq!(*runtime.writes.lock().unwrap(), 0);
}

#[test]
fn an_invalid_level_is_refused_before_anything_is_recorded() {
    let persistence = Arc::new(RecordedDefaults::default());
    let use_case = use_case_persisting(persistence.clone());
    let mut runtime = FakeRuntime::default();
    let error = use_case
        .execute(
            &mut runtime,
            &EffortChangeRequest {
                model: "xai/grok-4.5".into(),
                level: "xhigh".into(),
                persist: Some(DefaultScope::Local),
            },
        )
        .unwrap_err();
    assert!(matches!(error, EffortChangeError::Unsupported { .. }));
    assert!(persistence.records.lock().unwrap().is_empty());
}
