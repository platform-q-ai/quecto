use super::*;

/// One change that must make valid fields invalid.
type Change = Box<dyn Fn(&mut RunFields)>;
use crate::domain::project_board::services::fixtures_tests::{at, open_run, plan};
use crate::domain::project_board::value_objects::vocabulary::PlanState;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn ended() -> RunFields {
    let mut fields = open_run(1, 1).into_fields();
    fields.ended = Some(at("09:30:00"));
    fields.outcome = Some(RunOutcome::Succeeded);
    fields.cost = Some(1_000);
    fields.items = vec![RunItem {
        id: slug("schema"),
        state: ItemState::Done,
        owner: Some("worker-1".into()),
        evidence: vec!["PR #7".into()],
    }];
    fields
}

#[test]
fn an_open_run_and_one_folded_back_build() {
    assert!(open_run(1, 1).is_open());
    let run = Run::new(ended()).unwrap();
    assert!(!run.is_open());
    assert_eq!(run.fields(), &ended());
}

#[test]
fn fields_that_break_a_rule_build_no_run() {
    let cases: Vec<(&str, Change)> = vec![
        ("number", Box::new(|r| r.number = 0)),
        ("number", Box::new(|r| r.number = MAX_RUN_NUMBER + 1)),
        ("plan_version", Box::new(|r| r.plan_version = 0)),
        ("ended", Box::new(|r| r.outcome = None)),
        ("ended", Box::new(|r| r.ended = None)),
        ("ended", Box::new(|r| r.ended = Some(at("08:59:59")))),
        ("cost", Box::new(|r| r.cost = Some(MAX_BUDGET_TOKENS + 1))),
        ("items", Box::new(|r| r.items.push(r.items[0].clone()))),
        (
            "items/0/owner",
            Box::new(|r| r.items[0].owner = Some(String::new())),
        ),
        (
            "items/0/evidence",
            Box::new(|r| r.items[0].evidence = vec!["x".into(), "x".into()]),
        ),
    ];
    for (field, change) in cases {
        let mut broken = ended();
        change(&mut broken);
        let error = Run::new(broken).expect_err(field);
        assert_eq!(error.field, field, "{error:?}");
    }
    let mut early = open_run(1, 1).into_fields();
    early.cost = Some(1);
    assert_eq!(
        Run::new(early).unwrap_err().field,
        "items",
        "a cost waits for the end"
    );
}

#[test]
fn a_run_ran_its_own_plan_version_and_its_items() {
    let run = Run::new(ended()).unwrap();
    assert_eq!(run.ran(&plan(1, PlanState::Accepted)), Ok(()));
    assert_eq!(
        run.ran(&plan(2, PlanState::Accepted)).unwrap_err().field,
        "plan_version"
    );
    let mut stranger = ended();
    stranger.items[0].id = slug("ghost");
    let stranger = Run::new(stranger).unwrap();
    assert_eq!(
        stranger
            .ran(&plan(1, PlanState::Accepted))
            .unwrap_err()
            .field,
        "items/0/id"
    );
    let mut elsewhere = ended();
    elsewhere.task = slug("other");
    let elsewhere = Run::new(elsewhere).unwrap();
    assert_eq!(
        elsewhere
            .ran(&plan(1, PlanState::Accepted))
            .unwrap_err()
            .field,
        "task"
    );
}
