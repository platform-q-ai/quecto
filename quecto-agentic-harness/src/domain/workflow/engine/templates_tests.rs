use super::*;
use crate::domain::workflow::{WorkflowConfig, WorkflowEngine};
use std::collections::HashSet;

#[test]
fn built_in_default_templates_include_generic_workflows() {
    let templates = default_templates();
    let ids = template_ids(&templates);

    assert_eq!(
        ids,
        HashSet::from(["investigate", "chore", "bugfix", "feature", "refactor",]),
    );
}

#[test]
fn built_in_default_templates_are_valid_and_usable_by_engine() {
    let engine = WorkflowEngine::new(WorkflowConfig::default(), false)
        .expect("bundled generic workflow templates must validate");

    assert_eq!(
        template_ids(&default_templates()),
        engine
            .list_templates()
            .into_iter()
            .map(|template| template.id)
            .collect::<HashSet<_>>()
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>(),
    );
}

#[test]
fn built_in_default_templates_have_complete_generic_content() {
    for template in default_templates() {
        assert!(!template.label.trim().is_empty(), "{} label", template.id);
        assert!(
            !template.description.trim().is_empty(),
            "{} description",
            template.id,
        );
        assert!(
            template
                .when_to_use
                .as_deref()
                .is_some_and(|when_to_use| !when_to_use.trim().is_empty()),
            "{} when_to_use",
            template.id,
        );
        assert!(!template.steps.is_empty(), "{} steps", template.id);

        for step in template.steps {
            assert!(!step.key.trim().is_empty(), "{} step key", template.id);
            assert!(!step.label.trim().is_empty(), "{} step label", template.id);
            assert!(!step.phase.trim().is_empty(), "{} step phase", template.id);
            assert!(
                step.guidance
                    .as_deref()
                    .is_some_and(|guidance| !guidance.trim().is_empty()),
                "{}:{} guidance",
                template.id,
                step.key,
            );
        }
    }
}

fn template_ids(templates: &[WorkflowTemplate]) -> HashSet<&str> {
    templates
        .iter()
        .map(|template| template.id.as_str())
        .collect()
}

// Current approved set: feature round 2, all other templates round 1. Historical
// fixtures stay immutable; compare full typed objects using crate-local files.
fn current_approved_candidates() -> Vec<WorkflowTemplate> {
    [
        include_str!("../../../../tests/fixtures/workflow-approved-round-1/investigate.json"),
        include_str!("../../../../tests/fixtures/workflow-approved-round-1/chore.json"),
        include_str!("../../../../tests/fixtures/workflow-approved-round-1/bugfix.json"),
        include_str!("../../../../tests/fixtures/workflow-approved-round-2/feature.json"),
        include_str!("../../../../tests/fixtures/workflow-approved-round-1/refactor.json"),
    ]
    .into_iter()
    .map(|json| {
        let mut original: serde_json::Value = serde_json::from_str(json).unwrap();
        let template: WorkflowTemplate = serde_json::from_value(original.clone()).unwrap();
        // Empty guards are omitted by Serde. All other fields in these full
        // candidates must survive; unknown/misspelled fields cannot disappear.
        if original.get("guards") == Some(&serde_json::json!([])) {
            original.as_object_mut().unwrap().remove("guards");
        }
        assert_eq!(serde_json::to_value(&template).unwrap(), original);
        template
    })
    .collect()
}

#[test]
fn approved_candidates_match_complete_source_templates() {
    let source = default_templates();
    let candidates = current_approved_candidates();
    assert_eq!(template_ids(&source), template_ids(&candidates));
    for candidate in candidates {
        let actual = source.iter().find(|t| t.id == candidate.id).unwrap();
        assert_eq!(
            actual, &candidate,
            "source differs from approved {}",
            candidate.id
        );
    }
}

#[test]
fn approved_candidates_roundtrip_and_bind_without_content_loss() {
    use crate::domain::workflow::{MAX_WORKFLOW_SPEC_BYTES, WorkflowMode, WorkflowSpec};

    let candidates = current_approved_candidates();
    WorkflowEngine::new(
        WorkflowConfig {
            templates: candidates.clone(),
            ..WorkflowConfig::default()
        },
        false,
    )
    .unwrap();
    for candidate in candidates {
        let spec = WorkflowSpec {
            template: candidate.clone(),
        };
        let bytes = serde_json::to_vec(&spec).unwrap();
        assert!(bytes.len() <= MAX_WORKFLOW_SPEC_BYTES);
        let decoded: WorkflowSpec = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, spec);
        let mut engine = WorkflowEngine::new(
            WorkflowConfig {
                templates: vec![decoded.template],
                ..WorkflowConfig::default()
            },
            false,
        )
        .unwrap();
        engine.select_template(&candidate.id, None).unwrap();
        engine.set_bound(true);
        assert!(engine.is_bound());
        assert_eq!(engine.mode(), WorkflowMode::Active);
        assert_eq!(engine.list_templates().len(), 1);
        assert_eq!(engine.active_template(), Some(&candidate));
        assert!(
            engine
                .select_template("not-the-assigned-template", None)
                .is_err()
        );
        engine.check(1).unwrap();
        engine.reset();
        assert!(engine.is_bound());
        assert_eq!(engine.mode(), WorkflowMode::Active);
        assert_eq!(engine.active_template(), Some(&candidate));
        assert!(engine.all_step_statuses().iter().all(|step| !step.done));
    }
}

#[test]
fn approved_candidates_validate_structure_and_reject_invalid_keys() {
    use crate::domain::workflow::WorkflowGuardRule;

    for candidate in current_approved_candidates() {
        assert!(!candidate.steps.is_empty());
        let keys: HashSet<_> = candidate.steps.iter().map(|s| &s.key).collect();
        assert_eq!(keys.len(), candidate.steps.len());
        for step in &candidate.steps {
            assert!(!step.key.trim().is_empty());
            assert!(!step.label.trim().is_empty());
            // Phases may repeat and are not an enum.
            assert!(!step.phase.trim().is_empty());
            assert!(
                step.guidance
                    .as_deref()
                    .is_some_and(|s| !s.trim().is_empty())
            );
        }
        let invalid = |template| {
            WorkflowEngine::new(
                WorkflowConfig {
                    templates: vec![template],
                    ..WorkflowConfig::default()
                },
                false,
            )
            .is_err()
        };
        let mut duplicate = candidate.clone();
        duplicate.steps.push(duplicate.steps[0].clone());
        assert!(invalid(duplicate));
        let mut blank = candidate.clone();
        blank.steps[0].key = " ".into();
        assert!(invalid(blank));
        let mut empty = candidate.clone();
        empty.steps.clear();
        assert!(invalid(empty));
        let mut bad_guard = candidate;
        bad_guard.guards.push(WorkflowGuardRule {
            commands: vec!["example publish".into()],
            before_step_key: "nonexistent-step".into(),
            message: "check prerequisites".into(),
        });
        assert!(invalid(bad_guard));
    }
}
