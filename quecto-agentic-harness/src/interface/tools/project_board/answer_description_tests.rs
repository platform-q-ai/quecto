//! A task's answer carries its description's sections beside the text
//! (#2705 T2): additive fields only, legacy text flagged `needs_goal`.
use super::*;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};

fn task(description: &str) -> Task {
    let at = Timestamp::parse("2026-10-10T08:00:00Z").unwrap();
    Task::new(TaskFields {
        id: Slug::parse("a").unwrap(),
        title: "Task a".into(),
        kind: TaskKind::Task,
        description: description.into(),
        status: TaskStatus::Draft,
        parent: None,
        depends_on: vec![],
        accepted_plan: None,
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at.clone(),
        updated: at,
        description_form: Default::default(),
        reason: None,
    })
    .unwrap()
}

#[test]
fn a_sectioned_description_answers_its_sections_beside_its_text() {
    let text = "## Goal\nShip it.\n\n## Done when\n- [ ] Merged\n- [x] CI green\n\n## Links\n- [PR #1](https://x.io/1)";
    let answered = task_json(&task(text));
    assert_eq!(answered["description"], text);
    assert_eq!(answered["goal"], "Ship it.");
    assert_eq!(answered["why"], Value::Null);
    assert_eq!(
        answered["done_when"],
        json!([{"outcome": "Merged", "done": false}, {"outcome": "CI green", "done": true}])
    );
    assert_eq!(answered["links"], json!(["[PR #1](https://x.io/1)"]));
    assert_eq!(answered["needs_goal"], false);
}

#[test]
fn legacy_text_answers_as_it_is_and_needs_a_goal() {
    for text in ["Free-form markdown.\n\n- a list", ""] {
        let answered = task_json(&task(text));
        assert_eq!(answered["description"], text);
        assert_eq!(answered["needs_goal"], true);
        assert_eq!(answered["goal"], Value::Null);
        assert_eq!(answered["why"], Value::Null);
        assert_eq!(answered["done_when"], json!([]));
        assert_eq!(answered["links"], json!([]));
    }
}
