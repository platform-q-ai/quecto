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
