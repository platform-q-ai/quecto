//! #2481 M4b review round 2: a plan's text as a whole is held to its
//! limit by the domain alone.
use super::*;
use crate::domain::project_board::services::fixtures_tests::{at, person, task};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus::InProgress;

/// The plan's text as a whole is the domain's one check (#2481 M4b
/// review round 2): a proposal of exactly the most a plan holds is taken,
/// and one byte more is refused as `text`.
#[test]
fn a_plans_text_is_held_to_the_most_a_plan_holds_exactly() {
    use crate::domain::project_board::entities::plan::{MAX_APPROACH_CHARS, MAX_PLAN_TEXT_BYTES};
    use crate::domain::project_board::entities::task_parts::{MAX_LINE_CHARS, MAX_PER_ITEM};
    // Task items whose titles and criteria hold `bytes` bytes of text.
    let items_of = |mut bytes: usize| {
        let mut items = vec![];
        while bytes > 0 {
            // A line of the letter `letter`, so an item's criteria differ.
            let take = |bytes: &mut usize, letter: u8| {
                let line = (*bytes).min(MAX_LINE_CHARS);
                *bytes -= line;
                char::from(letter).to_string().repeat(line)
            };
            let title = take(&mut bytes, b'z');
            let criteria: Vec<String> = (0..MAX_PER_ITEM)
                .map(|at| take(&mut bytes, b'a' + u8::try_from(at).unwrap()))
                .filter(|line| !line.is_empty())
                .collect();
            items.push(PlanItem {
                id: Slug::parse(&format!("i{}", items.len())).unwrap(),
                title,
                criteria,
                depends_on: vec![],
            });
        }
        items
    };
    let approach = "\u{1f600}".repeat(MAX_APPROACH_CHARS);
    let rest = MAX_PLAN_TEXT_BYTES - approach.len();
    let content = |bytes| PlanContent {
        handoff: None,
        entry: PlanEntry::Legacy,
        approach: approach.clone(),
        items: items_of(bytes),
        team: None,
    };
    let mut fields = task(InProgress).into_fields();
    fields.accepted_plan = None;
    let held = Task::new(fields).unwrap();
    let ada = person("Ada");
    let most = propose_at(
        &held,
        &[],
        content(rest),
        &ada,
        &Moment {
            now: at("10:00:00"),
            latest: None,
        },
    )
    .unwrap();
    assert_eq!(most.fields().text_bytes(), MAX_PLAN_TEXT_BYTES);
    match propose_at(
        &held,
        &[],
        content(rest + 1),
        &ada,
        &Moment {
            now: at("10:00:00"),
            latest: None,
        },
    ) {
        Err(BoardRuleError::InvalidPlan(error)) => assert_eq!(error.field, "text"),
        other => panic!("{other:?}"),
    }
}

/// A task item `id`, its title `title_chars` long, with `count` criteria
/// of `chars` characters each.
fn sized_item(id: &str, title_chars: usize, count: usize, chars: usize) -> PlanItem {
    PlanItem {
        id: Slug::parse(id).unwrap(),
        title: "t".repeat(title_chars),
        criteria: (0..count)
            .map(|n| format!("{n:>chars$}").replace(' ', "c"))
            .collect(),
        depends_on: vec![],
    }
}

/// The step rules (#2705) on entry: a capped proposal's task items are
/// held to them; a legacy one keeps the schema's limits.
#[test]
fn a_capped_proposal_holds_its_items_to_the_step_rules() {
    use crate::domain::project_board::entities::plan::PlanEntry::{Capped, Legacy};
    let mut fields = task(InProgress).into_fields();
    fields.accepted_plan = None;
    let held = Task::new(fields).unwrap();
    let propose = |entry, items| {
        let content = PlanContent {
            handoff: Some("The detail.".into()),
            entry,
            approach: "Outline.".into(),
            items,
            team: None,
        };
        let now = Moment {
            now: at("10:00:00"),
            latest: None,
        };
        propose_at(&held, &[], content, &person("Ada"), &now)
    };
    let field = |answer: Result<Plan, BoardRuleError>| match answer {
        Err(BoardRuleError::InvalidPlan(error)) => error.field,
        other => panic!("{other:?}"),
    };
    let short = sized_item("short", 70, 4, 150);
    assert!(propose(Capped, vec![short.clone()]).is_ok());
    let long_title = sized_item("long", 71, 1, 10);
    let fields = [
        (vec![short.clone(), long_title.clone()], "items/1/title"),
        (vec![sized_item("none", 10, 0, 1)], "items/0/criteria"),
        (vec![sized_item("five", 10, 5, 10)], "items/0/criteria"),
        (vec![sized_item("wide", 10, 2, 151)], "items/0/criteria/0"),
    ];
    for (items, want) in fields {
        assert_eq!(field(propose(Capped, items.clone())), want);
        assert!(propose(Legacy, items).is_ok(), "{want} enters legacy");
    }
    let legacy = sized_item("legacy", 500, 16, 500);
    assert!(propose(Legacy, vec![legacy]).is_ok());
}

/// A draft entered before the step rules is edited capped (the agent
/// tool) without being rewritten: each task item it holds as it is
/// enters nothing, and a task item given anew is held to the rules.
#[test]
fn a_capped_edit_keeps_the_items_a_legacy_draft_holds() {
    use crate::domain::project_board::entities::plan::PlanEntry::Capped;
    let legacy = sized_item("legacy", 500, 16, 500);
    let mut fields = crate::domain::project_board::services::fixtures_tests::plan(
        2,
        crate::domain::project_board::value_objects::vocabulary::PlanState::Draft,
    )
    .into_fields();
    fields.items = vec![legacy.clone()];
    let draft = Plan::new(fields).unwrap();
    let held = task(InProgress);
    let edit = |items: Vec<PlanItem>, team| {
        let content = PlanContent {
            handoff: None,
            entry: Capped,
            approach: draft.fields().approach.clone(),
            items,
            team,
        };
        edit_draft(
            DraftOf {
                task: &held,
                versions: std::slice::from_ref(&draft),
                plan: &draft,
            },
            content,
            None,
            &person("Ada"),
            &Moment {
                now: at("10:00:00"),
                latest: None,
            },
        )
    };
    let kept = edit(vec![legacy.clone()], None).expect("its items kept");
    assert_eq!(kept.fields().items, vec![legacy.clone()]);
    let short = sized_item("short", 70, 1, 150);
    let added = edit(vec![legacy.clone(), short], draft.fields().team.clone());
    assert_eq!(added.expect("a short item added").fields().items.len(), 2);
    // Held at another index, after a new item: kept by its content.
    let first = sized_item("first", 70, 1, 150);
    let moved = edit(vec![first.clone(), legacy.clone()], None);
    assert_eq!(
        moved.expect("a held item moved").fields().items,
        vec![first.clone(), legacy.clone()]
    );
    // Its id and dependencies changed, its text as held: kept.
    let mut relinked = legacy.clone();
    relinked.id = Slug::parse("renamed").unwrap();
    relinked.depends_on = vec![first.id.clone()];
    let answer = edit(vec![first.clone(), relinked.clone()], None);
    assert_eq!(answer.expect("relinked").fields().items[1], relinked);
    // A changed title or criterion enters anew.
    let mut retitled = legacy.clone();
    retitled.title.pop();
    match edit(vec![retitled], None) {
        Err(BoardRuleError::InvalidPlan(error)) => assert_eq!(error.field, "items/0/title"),
        other => panic!("a changed title enters anew: {other:?}"),
    }
    let mut changed = legacy.clone();
    changed.criteria.pop();
    match edit(vec![changed], draft.fields().team.clone()) {
        Err(BoardRuleError::InvalidPlan(error)) => assert_eq!(error.field, "items/0/title"),
        other => panic!("a changed item enters anew: {other:?}"),
    }
}
