use super::*;

/// One change that must make valid fields invalid.
type Change = Box<dyn Fn(&mut PlanFields)>;
use crate::domain::project_board::services::fixtures_tests::{at, plan, team};

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
        (
            "team/deadline",
            Box::new(|p| p.team.as_mut().unwrap().deadline = Some(at("08:30:00"))),
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
