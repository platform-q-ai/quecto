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
