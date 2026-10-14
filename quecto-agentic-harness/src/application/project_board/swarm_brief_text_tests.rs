//! S4c: a brief rendered as its coordinator's task text, golden, and
//! always within the room its size allowed.
use super::super::{BriefItem, RUN_CRITERIA, RoleChoice, SwarmBrief};
use super::render;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::Effort;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn brief(items: Vec<BriefItem>) -> SwarmBrief {
    SwarmBrief {
        task: slug("board-store"),
        plan_version: 2,
        goal: "Board task board-store, plan v2: Board store".into(),
        constraints: vec![
            "Spike first.\nThen build.".into(),
            "The full plan is on the project board (task board-store, plan v2); \
             ask your parent for any part cut here."
                .into(),
        ],
        approach_cut: false,
        criteria: RUN_CRITERIA.to_vec(),
        member_limit: Some(3),
        deadline_in_seconds: 86_400,
        deadline_clamped: false,
        usage_budget: Some(2_000_000),
        items,
        coordinator: Some(RoleChoice {
            model: "claude-opus".into(),
            effort: Effort::High,
        }),
        not_applied: vec![],
    }
}

fn item(id: &str, acceptance: &[&str], dependencies: &[&str]) -> BriefItem {
    BriefItem {
        id: slug(id),
        title: format!("{id}: Build the {id}"),
        acceptance: acceptance.iter().map(|line| (*line).to_owned()).collect(),
        dependencies: dependencies.iter().map(|id| slug(id)).collect(),
    }
}

#[test]
fn a_brief_renders_as_its_golden_text() {
    let text = render(&brief(vec![
        item("schema", &[], &[]),
        item("api", &["serves GET /tasks", "answers 404"], &["schema"]),
    ]));
    let golden = "\
You coordinate the swarm for board task board-store, plan version 2. Create this swarm run and its task items exactly as given here, then run it as your playbook says. Copy every value as written.

Run (op=create):
goal: Board task board-store, plan v2: Board store
member_limit: 3
deadline_in_seconds: 86400
criteria:
- id task_items, kind review: Every task item is completed and accepted against its acceptance criteria.
- id delivery_checks, kind review: The integrated result passes the project's own checks, as its contributing guide or CI defines them.
constraint 1 (until the next label):
Spike first.
Then build.
constraint 2 (until the next label):
The full plan is on the project board (task board-store, plan v2); ask your parent for any part cut here.

Budget (op=usage_budget): token_limit 2000000

Task items (op=task_item_create), in this order. Give each title exactly as written: the board matches each item back by it. For dependencies, pass the task_item_id the create of each named item returned. Each item's acceptance lines run until its dependencies line.
item schema
title: schema: Build the schema
acceptance:
dependencies: none
item api
title: api: Build the api
acceptance:
serves GET /tasks
answers 404
dependencies: schema

When the run ends, report its summary to your parent.
";
    assert_eq!(text, golden);
}

/// No team: the coordinator sizes the pool, and no budget is set.
#[test]
fn a_brief_with_no_team_leaves_the_pool_and_budget_alone() {
    let mut teamless = brief(vec![item("schema", &["a"], &[])]);
    teamless.member_limit = None;
    teamless.usage_budget = None;
    let text = render(&teamless);
    assert!(text.contains("member_limit: yours to choose (the plan names no team)\n"));
    assert!(!text.contains("usage_budget"), "{text}");
}

/// Items at their widest (sixteen acceptance lines of 500 characters and
/// sixteen dependencies each) still render within the room the brief's
/// size allowed: the renderer asserts it.
#[test]
fn the_widest_items_render_within_their_room() {
    let names: Vec<String> = (0..16).map(|at| format!("dep-{at:02}")).collect();
    let line = "x".repeat(500);
    let acceptance: Vec<&str> = (0..16).map(|_| line.as_str()).collect();
    let dependencies: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut items: Vec<BriefItem> = names.iter().map(|name| item(name, &[], &[])).collect();
    items.extend((0..8).map(|at| {
        let mut wide = item(&format!("wide-{at}"), &acceptance, &dependencies);
        wide.title = format!("wide-{at}: {}", "t".repeat(400));
        wide
    }));
    let widest = brief(items);
    let text = render(&widest);
    assert!(
        text.len() <= widest.bytes(),
        "{} > {}",
        text.len(),
        widest.bytes()
    );
}
