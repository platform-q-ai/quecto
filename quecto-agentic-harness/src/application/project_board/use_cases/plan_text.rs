//! A plan's text as it enters the board (#2481 M4b): its approach as
//! markdown ([`board_text::text`]), each task item's title and criteria as
//! lines ([`board_text::line`]); and what the normaliser changed, told by
//! kind, never as text.
use crate::application::project_board::telemetry::OpFacts;
use crate::domain::project_board::entities::plan::{Plan, PlanItem};
use crate::domain::project_board::value_objects::board_text;
use crate::domain::project_board::value_objects::telemetry::{Normalised, TextChange};

/// `items` with their titles and criteria as the board keeps them; ids
/// and dependencies as given.
pub(super) fn entered_items(items: &[PlanItem]) -> Vec<PlanItem> {
    items
        .iter()
        .map(|item| PlanItem {
            id: item.id.clone(),
            title: board_text::line(&item.title),
            criteria: item
                .criteria
                .iter()
                .map(|line| board_text::line(line))
                .collect(),
            depends_on: item.depends_on.clone(),
        })
        .collect()
}

/// A handoff (#2705 T3) as the board keeps it: normalised as markdown, or
/// none when nothing is left.
pub(super) fn handoff(raw: &str) -> Option<String> {
    Some(board_text::text(raw)).filter(|text| !text.is_empty())
}

/// What the normaliser changes in the plan text a request gives: in its
/// approach, and in any of its task items' titles and criteria (each kind
/// once, in [`TextChange`]'s order). A field not given tells nothing.
pub(super) fn normalised(approach: Option<&str>, items: Option<&[PlanItem]>) -> Normalised {
    let items_changes = |items: &[PlanItem]| {
        let told: Vec<TextChange> = items
            .iter()
            .flat_map(|item| std::iter::once(&item.title).chain(&item.criteria))
            .flat_map(|line| board_text::line_changes(line))
            .collect();
        TextChange::ALL
            .iter()
            .copied()
            .filter(|change| told.contains(change))
            .collect()
    };
    Normalised {
        title: None,
        description: None,
        approach: approach.map(board_text::text_changes),
        items: items.map(items_changes),
    }
}

/// Notes in `facts` the plan a change wrote: its version, and how many
/// task items and team roles it holds.
pub(super) fn note_written(facts: &mut OpFacts, plan: &Plan) {
    let count = |items: usize| u64::try_from(items).unwrap_or(u64::MAX);
    let plan = plan.fields();
    facts.plan_version = Some(u64::from(plan.version));
    facts.items = Some(count(plan.items.len()));
    facts.roles = Some(count(plan.team.as_ref().map_or(0, |team| team.roles.len())));
}
