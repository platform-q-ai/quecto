use super::*;

/// One change that must make valid fields invalid.
type Change = Box<dyn Fn(&mut PlanFields)>;
use crate::domain::project_board::entities::fixtures_tests::{at, plan, team};
use crate::domain::project_board::entities::task_parts::{MAX_ITEMS, MAX_LINE_CHARS, MAX_PER_ITEM};

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn fields() -> PlanFields {
    plan(1, PlanState::Draft).into_fields()
}

#[test]
fn a_valid_plan_builds_and_reads_back_unchanged() {
    let built = Plan::new(fields()).unwrap();
    assert_eq!(built.fields(), &fields());
    assert_eq!(
        built.item(&slug("api")).unwrap().depends_on,
        vec![slug("schema")]
    );
    assert_eq!(built.item(&slug("nope")), None);
}

#[test]
fn fields_that_break_a_rule_build_no_plan() {
    let cases: Vec<(&str, Change)> = vec![
        ("version", Box::new(|p| p.version = 0)),
        ("version", Box::new(|p| p.version = MAX_PLAN_VERSION + 1)),
        ("updated", Box::new(|p| p.updated = at("08:00:00"))),
        ("approach", Box::new(|p| p.approach = "a\u{0}b".into())),
        (
            "approach",
            Box::new(|p| p.approach = "é".repeat(MAX_APPROACH_CHARS + 1)),
        ),
        ("items", Box::new(|p| p.items[1].id = slug("schema"))),
        (
            "items/0/title",
            Box::new(|p| p.items[0].title = String::new()),
        ),
        (
            "items/0/criteria",
            Box::new(|p| p.items[0].criteria = vec!["x".into(), "x".into()]),
        ),
        (
            "items/1/depends_on",
            Box::new(|p| p.items[1].depends_on = vec![slug("ghost")]),
        ),
        (
            "items/1/depends_on",
            Box::new(|p| p.items[1].depends_on = vec![slug("api")]),
        ),
        (
            "items/0/depends_on",
            Box::new(|p| p.items[0].depends_on = vec![slug("api")]),
        ),
        (
            "team/roles",
            Box::new(|p| p.team.as_mut().unwrap().roles.clear()),
        ),
    ];
    for (field, change) in cases {
        let mut broken = fields();
        change(&mut broken);
        let error = Plan::new(broken).expect_err(field);
        assert_eq!(error.field, field, "{error:?}");
    }
}

#[test]
fn every_bound_admits_exactly_its_limit() {
    let mut widest = fields();
    widest.version = MAX_PLAN_VERSION;
    widest.approach = "é".repeat(MAX_APPROACH_CHARS);
    widest.team = Some(team());
    assert!(Plan::new(widest).is_ok());
    let mut no_team = fields();
    no_team.team = None;
    no_team.items.clear();
    assert!(
        Plan::new(no_team).is_ok(),
        "a plan may name no items or team"
    );
}

/// Who accepted a plan, and when, is recorded from its acceptance on and
/// never before; who proposed it is always a valid identity.
#[test]
fn an_acceptance_is_recorded_only_once_the_plan_was_accepted() {
    let accepted = || plan(1, PlanState::Accepted).into_fields();
    let cases: Vec<(&str, Change)> = vec![
        (
            "accepted",
            Box::new(|p| p.accepted = Some((p.proposed_by.clone(), at("08:30:00")))),
        ),
        (
            "proposed_by/email",
            Box::new(|p| p.proposed_by.email = "ada".into()),
        ),
    ];
    for (field, change) in cases {
        let mut broken = fields();
        change(&mut broken);
        assert_eq!(Plan::new(broken).unwrap_err().field, field);
    }
    let mut unrecorded = accepted();
    unrecorded.accepted = None;
    assert_eq!(Plan::new(unrecorded).unwrap_err().field, "accepted");
    let mut early = accepted();
    early.accepted = Some((early.proposed_by.clone(), at("08:00:00")));
    assert_eq!(Plan::new(early).unwrap_err().field, "accepted/at");
    let mut superseded = accepted();
    superseded.state = PlanState::Superseded;
    assert!(Plan::new(superseded).is_ok(), "a superseded plan keeps it");
}

/// The items are capped at their limit, and a plan's text at its budget,
/// exactly.
#[test]
fn the_items_and_the_text_stop_exactly_at_their_limits() {
    let item = |n: usize| PlanItem {
        id: slug(&format!("i{n}")),
        title: format!("Item {n}"),
        criteria: vec![],
        depends_on: vec![],
    };
    let mut full = fields();
    full.items = (0..MAX_ITEMS).map(item).collect();
    assert!(Plan::new(full.clone()).is_ok(), "{MAX_ITEMS} items");
    full.items.push(item(MAX_ITEMS));
    assert_eq!(Plan::new(full).unwrap_err().field, "items");

    let mut wide = fields();
    wide.team = None;
    wide.items = vec![item(0)];
    wide.approach = "\u{1d11e}".repeat(MAX_APPROACH_CHARS);
    let mut left = MAX_PLAN_TEXT_BYTES - wide.text_bytes();
    let mut n = 0;
    while left > 0 {
        let len = left.min(MAX_LINE_CHARS);
        let number = format!("{n}-");
        let line = format!("{number}{}", "x".repeat(len.saturating_sub(number.len())));
        let line = match line.len() == len {
            true => line,
            false => "y".repeat(len),
        };
        let at = n / MAX_PER_ITEM;
        if wide.items.len() <= at {
            wide.items.push(item(at));
            left -= wide.items[at].title.len();
            continue;
        }
        wide.items[at].criteria.push(line);
        left -= len;
        n += 1;
    }
    assert_eq!(wide.text_bytes(), MAX_PLAN_TEXT_BYTES);
    let exact = Plan::new(wide.clone());
    assert!(exact.is_ok(), "exactly the budget: {:?}", exact.err());
    wide.items[0].title.push('x');
    assert_eq!(Plan::new(wide).unwrap_err().field, "text", "a byte past it");
}

/// A deadline is informational: one already past when the plan was made
/// is allowed, so proposing and editing agree.
#[test]
fn a_past_deadline_is_allowed() {
    let mut past = fields();
    past.team.as_mut().unwrap().deadline = Some(at("07:00:00"));
    assert!(Plan::new(past).is_ok());
}
