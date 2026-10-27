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

/// The dependency refusals a person or agent reads say "task items".
#[test]
fn dependency_refusals_name_task_items() {
    let refusal = |change: Change| {
        let mut broken = fields();
        change(&mut broken);
        Plan::new(broken).unwrap_err().problem
    };
    assert_eq!(
        refusal(Box::new(|p| p.items[1].depends_on = vec![slug("ghost")])),
        "must name other task items of this plan"
    );
    assert_eq!(
        refusal(Box::new(|p| p.items[0].depends_on = vec![slug("api")])),
        "is on or behind a cycle of task item dependencies"
    );
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
        (
            "edited_by/email",
            Box::new(|p| {
                let mut editor = p.proposed_by.clone();
                editor.email = "bob".into();
                p.edited_by = Some(editor);
            }),
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

/// A deadline is a duration from Start, so it never passes before the
/// plan starts: one from ten minutes to seven days is a plan's, and one
/// outside them is refused as the plan is made.
#[test]
fn a_deadline_is_seconds_from_start_within_its_bounds() {
    use crate::domain::project_board::entities::team::{
        MAX_DEADLINE_SECONDS, MIN_DEADLINE_SECONDS,
    };
    assert_eq!((MIN_DEADLINE_SECONDS, MAX_DEADLINE_SECONDS), (600, 604_800));
    for (seconds, kept) in [
        (MIN_DEADLINE_SECONDS, true),
        (MAX_DEADLINE_SECONDS, true),
        (MIN_DEADLINE_SECONDS - 1, false),
        (MAX_DEADLINE_SECONDS + 1, false),
    ] {
        let mut timed = fields();
        timed.team.as_mut().unwrap().deadline_seconds = Some(seconds);
        let made = Plan::new(timed).map_err(|error| error.field);
        let want = match kept {
            true => Ok(()),
            false => Err("team/deadline_seconds".to_string()),
        };
        assert_eq!(made.map(|_| ()), want, "{seconds}");
    }
}

/// A task item `title`d, with `criteria`.
fn step(title: &str, criteria: Vec<String>) -> PlanItem {
    PlanItem {
        id: slug("step"),
        title: title.into(),
        criteria,
        depends_on: vec![],
    }
}

/// `count` distinct criteria of `chars` characters each.
fn criteria(count: usize, chars: usize) -> Vec<String> {
    (0..count)
        .map(|n| {
            let number = format!("{n} ");
            format!("{number}{}", "é".repeat(chars - number.chars().count()))
        })
        .collect()
}

/// The field and the problem `item` is refused with, entering capped.
fn refused(item: &PlanItem) -> (String, String) {
    let error = PlanEntry::Capped
        .item("items/3", item)
        .expect_err("refused on entry");
    (error.field, error.problem)
}

/// The owner-approved step rules (#2705): a capped task item's title is
/// one line of at most 70 characters, and it has 1 to 4 criteria of at
/// most 150 each; past them, the refusal names the field and sends the
/// detail to the handoff.
#[test]
fn a_capped_item_enters_with_a_short_title_and_one_to_four_short_criteria() {
    let rules = &PLAN_ENTRY_RULES;
    assert_eq!(
        (
            rules.max_item_title_chars,
            rules.item_criteria.clone(),
            rules.max_criterion_chars
        ),
        (70, 1..=4, 150)
    );
    let widest = step(&"é".repeat(70), criteria(4, 150));
    assert_eq!(PlanEntry::Capped.item("items/3", &widest), Ok(()));
    let narrowest = step("Done", criteria(1, 2));
    assert_eq!(PlanEntry::Capped.item("items/3", &narrowest), Ok(()));

    let long_title = step(&"é".repeat(71), criteria(1, 10));
    assert_eq!(
        refused(&long_title),
        (
            "items/3/title".to_string(),
            format!("is 71 characters; the most is 70: {STATE_THE_OUTCOME}")
        )
    );
    for count in [0, 5] {
        assert_eq!(
            refused(&step("Done", criteria(count, 10))),
            (
                "items/3/criteria".to_string(),
                format!("has {count} criteria; it takes 1 to 4: {MOVE_STEP_DETAIL}")
            ),
            "{count} criteria"
        );
    }
    let mut long_criterion = criteria(3, 150);
    long_criterion[2].push('x');
    assert_eq!(
        refused(&step("Done", long_criterion)),
        (
            "items/3/criteria/2".to_string(),
            format!("is 151 characters; the most is 150: {MOVE_STEP_DETAIL}")
        )
    );
}

/// A plan entered before the step rules, or legacy entry, is held to the
/// schema alone: a long title and 16 criteria of 500 build a plan, and
/// enter legacy.
#[test]
fn a_legacy_item_keeps_the_schemas_limits() {
    let legacy = step(
        &"t".repeat(MAX_LINE_CHARS),
        criteria(MAX_PER_ITEM, MAX_LINE_CHARS),
    );
    assert_eq!(PlanEntry::Legacy.item("items/0", &legacy), Ok(()));
    let mut stored = fields();
    stored.items = vec![legacy.clone()];
    let built = Plan::new(stored).expect("a legacy plan builds");
    assert_eq!(built.fields().items, vec![legacy.clone()]);
    assert_eq!(
        PlanEntry::Capped
            .item("items/0", &legacy)
            .unwrap_err()
            .field,
        "items/0/title"
    );
}
