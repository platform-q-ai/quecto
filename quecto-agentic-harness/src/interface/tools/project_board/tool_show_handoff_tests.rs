//! A plan's handoff in the agent tool's `show` (#2705 T3): shown after
//! the approach when the plan has one (never a key otherwise, so a plan
//! without one shows as before); cut as the approach is when the answer
//! is past its cap, counted (`handoff_more`) and read on by pages
//! (`show(task, handoff_from: handoff_next)`).
use serde_json::{Value, json};

use super::{MAX_SHOW_BYTES, capped};
use crate::application::project_board::dto::ShownTask;
use crate::application::project_board::operations::{BoardOutcome, PlanPage, PlanView};
use crate::application::project_board::ports::BoardHead;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::plan::{MAX_HANDOFF_CHARS, Plan, PlanFields, PlanItem};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{PlanState, TaskKind, TaskStatus};
use crate::interface::tools::project_board::answer::outcome;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn at() -> Timestamp {
    Timestamp::parse("2026-10-10T09:00:00Z").unwrap()
}

/// Task `a` with its accepted plan v1, its handoff `handoff` and three
/// task items.
fn shown(handoff: Option<String>) -> ShownTask {
    let ada = Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    };
    let task = Task::new(TaskFields {
        id: slug("a"),
        title: "Task a".into(),
        kind: TaskKind::Task,
        description: String::new(),
        description_form: Default::default(),
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        accepted_plan: Some(1),
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at(),
        updated: at(),
    })
    .unwrap();
    let items = (0..3)
        .map(|index| PlanItem {
            id: slug(&format!("item-{index}")),
            title: format!("Item {index}"),
            criteria: vec![],
            depends_on: vec![],
        })
        .collect();
    let plan = Plan::new(PlanFields {
        task: slug("a"),
        version: 1,
        state: PlanState::Accepted,
        approach: "Outline.".into(),
        handoff,
        items,
        team: None,
        proposed_by: ada.clone(),
        edited_by: None,
        accepted: Some((ada, at())),
        created: at(),
        updated: at(),
    })
    .unwrap();
    ShownTask {
        head: BoardHead("0".repeat(40)),
        task,
        plans: vec![plan],
        runs: vec![],
        set_aside: vec![],
    }
}

fn answered(shown: ShownTask, page: Option<PlanPage>) -> Value {
    let view = PlanView {
        version: None,
        page,
    };
    capped(outcome(
        "show",
        &BoardOutcome::Shown(shown, view),
        None,
        None,
    ))
}

fn bytes(value: &Value) -> usize {
    value.to_string().len()
}

#[test]
fn a_plan_without_a_handoff_shows_no_handoff() {
    let answer = answered(shown(None), None);
    assert_eq!(answer["plan"].get("handoff"), None, "{answer}");
    let paged = answered(shown(None), PlanPage::paged(None, Some(0), None));
    assert_eq!(paged["plan"]["handoff"], Value::Null, "{paged}");
}

/// The longest handoff of quotes (each escaped) does not fit: the
/// approach stays whole, as much of the handoff as fits is shown with
/// what was left out counted, and pages from `handoff_next` read the
/// whole handoff back.
#[test]
fn a_long_handoff_is_cut_and_read_on_by_pages() {
    let handoff = "\"".repeat(MAX_HANDOFF_CHARS);
    let cut = answered(shown(Some(handoff.clone())), None);
    assert!(bytes(&cut) <= MAX_SHOW_BYTES, "{}", bytes(&cut));
    assert_eq!(cut["plan_truncated"], json!(true));
    assert_eq!(cut["plan"]["approach"], json!("Outline."));
    let first = cut["plan"]["handoff"].as_str().unwrap().to_string();
    assert!(!first.is_empty() && handoff.starts_with(&first));
    assert_eq!(
        cut["handoff_more"],
        json!(MAX_HANDOFF_CHARS - first.chars().count())
    );
    let mut read = first;
    let mut next = cut["handoff_next"].as_u64();
    let mut pages = 0;
    while let Some(from) = next {
        pages += 1;
        assert!(pages < 20, "pages always read on");
        let from = usize::try_from(from).unwrap();
        assert_eq!(from, read.chars().count());
        let page = answered(
            shown(Some(handoff.clone())),
            PlanPage::paged(None, Some(from), None),
        );
        assert!(bytes(&page) <= MAX_SHOW_BYTES, "{}", bytes(&page));
        assert_eq!(page["plan"]["handoff_from"], json!(from));
        read.push_str(page["plan"]["handoff"].as_str().unwrap());
        next = page["handoff_next"].as_u64();
    }
    assert_eq!(read, handoff);
}
