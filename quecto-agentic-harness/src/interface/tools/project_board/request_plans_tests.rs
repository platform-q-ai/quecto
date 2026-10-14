//! #2481 M4b: a plan's ops read their nested task items and team by
//! allowlist, and a fault names its path, recorded as its schema field.
use serde_json::{Value, json};

use super::super::{RequestError, parse};
use crate::application::project_board::dto::{PlanEdit, PlanProposal};
use crate::application::project_board::operations::BoardRequest;
use crate::domain::project_board::entities::plan::PlanItem;
use crate::domain::project_board::entities::team::{Budget, Role, RoleLimits, Team};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::Effort;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn refused(value: Value) -> RequestError {
    parse(&value).expect_err("refused")
}

/// The answer's field, and the record's schema field, fault and count.
fn fault(value: Value) -> (String, &'static str, &'static str, Option<u64>) {
    let error = refused(value);
    let recorded = error.fault;
    (
        error.field,
        recorded.field.as_str(),
        recorded.fault.as_str(),
        recorded.unexpected_count,
    )
}

fn team_json() -> Value {
    json!({
        "roles": [
            {"name": "coordinator", "model": "gpt-5.5", "effort": "high", "members": 1},
            {"name": "worker", "model": "gpt-5.5-mini", "effort": "low", "members": 3, "turns": 40},
        ],
        "budget_tokens": 2_000_000,
        "deadline": "2026-10-20T08:00:00Z",
        "image": null,
    })
}

fn team() -> Team {
    let role = |name: &str, model: &str, effort, members, turns| Role {
        name: slug(name),
        model: model.into(),
        effort,
        limits: RoleLimits { members, turns },
    };
    Team {
        roles: vec![
            role("coordinator", "gpt-5.5", Effort::High, 1, None),
            role("worker", "gpt-5.5-mini", Effort::Low, 3, Some(40)),
        ],
        budget: Some(Budget { tokens: 2_000_000 }),
        deadline: Some(Timestamp::parse("2026-10-20T08:00:00Z").unwrap()),
        image: None,
    }
}

#[test]
fn a_proposal_reads_its_items_and_team_as_show_gives_them() {
    let request = json!({
        "op": "propose_plan", "task": "a", "approach": "Spike.\r\n",
        "items": [
            {"id": "schema", "title": "Schema", "criteria": ["Migrates"], "depends_on": []},
            {"id": "api", "title": "API", "depends_on": ["schema"], "criteria": null},
        ],
        "team": team_json(),
    });
    let item = |id: &str, title: &str, criteria: &[&str], depends_on: &[&str]| PlanItem {
        id: slug(id),
        title: title.into(),
        criteria: criteria.iter().map(|line| line.to_string()).collect(),
        depends_on: depends_on.iter().map(|id| slug(id)).collect(),
    };
    assert_eq!(
        parse(&request),
        Ok(BoardRequest::ProposePlan(PlanProposal {
            task: slug("a"),
            approach: "Spike.\r\n".into(),
            items: vec![
                item("schema", "Schema", &["Migrates"], &[]),
                item("api", "API", &[], &["schema"]),
            ],
            team: Some(team()),
        })),
        "text as sent: the use case normalises it"
    );
    let teamless =
        json!({"op": "propose_plan", "task": "a", "approach": "", "items": [], "team": null});
    let Ok(BoardRequest::ProposePlan(proposal)) = parse(&teamless) else {
        panic!("a proposal with no team reads");
    };
    assert_eq!((proposal.items, proposal.team), (vec![], None));
}

/// An edit is a patch: a field left out is `None`, kept; `team: null`
/// takes the team away; and it sets at least one field.
#[test]
fn an_edit_reads_only_the_fields_it_gives() {
    let edit = |fields: Value| {
        let mut request = json!({"op": "edit_plan", "task": "a", "version": 2});
        request
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        parse(&request)
    };
    let none = PlanEdit {
        task: slug("a"),
        version: 2,
        approach: None,
        items: None,
        team: None,
        expected_updated: None,
    };
    assert_eq!(
        edit(json!({"items": [], "expected_updated": "2026-10-10T09:00:00Z"})),
        Ok(BoardRequest::EditPlan(PlanEdit {
            items: Some(vec![]),
            expected_updated: Some(Timestamp::parse("2026-10-10T09:00:00Z").unwrap()),
            ..none.clone()
        }))
    );
    assert_eq!(
        edit(json!({"team": null})),
        Ok(BoardRequest::EditPlan(PlanEdit {
            team: Some(None),
            ..none.clone()
        }))
    );
    assert_eq!(
        edit(json!({"approach": "New"})),
        Ok(BoardRequest::EditPlan(PlanEdit {
            approach: Some("New".into()),
            ..none
        }))
    );
    let nothing = edit(json!({"expected_updated": "2026-10-10T09:00:00Z"})).unwrap_err();
    assert_eq!(
        (nothing.field.as_str(), nothing.fault.fault.as_str()),
        ("op", "missing")
    );
}

/// A nested fault names its path in the answer and its schema field (the
/// path's first name) in the record; a stray nested key is `other`,
/// counted, never named.
#[test]
fn a_nested_fault_names_its_path_and_records_its_field() {
    let propose = |items: Value, team: Value| json!({"op": "propose_plan", "task": "a", "approach": "", "items": items, "team": team});
    let item = |extra: Value| {
        let mut item = json!({"id": "i", "title": "T"});
        item.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        item
    };
    let four = |fourth: Value| {
        let fine = item(json!({}));
        json!([fine.clone(), fine.clone(), fine, fourth])
    };
    let role = |extra: Value| {
        let mut role = json!({"name": "w", "model": "m", "effort": "low", "members": 1});
        role.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({"roles": [role]})
    };
    let at = |path: &str, field, fault| (path.to_string(), field, fault, None);
    let cases = [
        (
            propose(four(item(json!({"criteria": [7]}))), Value::Null),
            at("items/3/criteria/0", "items", "wrong_type"),
        ),
        (
            propose(
                four(item(json!({"depends_on": ["Not A Slug"]}))),
                Value::Null,
            ),
            at("items/3/depends_on/0", "items", "invalid_value"),
        ),
        (
            propose(json!([{"title": "T"}]), Value::Null),
            at("items/0/id", "items", "missing"),
        ),
        (
            propose(json!([7]), Value::Null),
            at("items/0", "items", "wrong_type"),
        ),
        (
            propose(json!({"id": "i"}), Value::Null),
            at("items", "items", "wrong_type"),
        ),
        (
            propose(
                json!([item(json!({"Secret Key": 1, "owner": 2}))]),
                Value::Null,
            ),
            (
                "items/0/Secret Key".to_string(),
                "other",
                "unexpected",
                Some(2),
            ),
        ),
        (
            propose(json!([]), json!({"roles": [], "budget": 5})),
            ("team/budget".to_string(), "other", "unexpected", Some(1)),
        ),
        (
            propose(json!([]), json!({"budget_tokens": 5})),
            at("team/roles", "team", "missing"),
        ),
        (
            propose(json!([]), role(json!({"effort": "extreme"}))),
            at("team/roles/0/effort", "team", "invalid_value"),
        ),
        (
            propose(json!([]), role(json!({"members": 70_000}))),
            at("team/roles/0/members", "team", "invalid_value"),
        ),
        (
            propose(json!([]), role(json!({"turns": "many"}))),
            at("team/roles/0/turns", "team", "wrong_type"),
        ),
        (
            propose(json!([]), json!({"roles": [], "deadline": "soon"})),
            at("team/deadline", "team", "invalid_value"),
        ),
        (
            json!({"op": "edit_plan", "task": "a", "version": 1000, "approach": ""}),
            at("version", "version", "invalid_value"),
        ),
        (
            json!({"op": "edit_plan", "task": "a", "version": "1", "approach": ""}),
            at("version", "version", "wrong_type"),
        ),
        (
            json!({"op": "propose_plan", "task": "a", "items": []}),
            at("approach", "approach", "missing"),
        ),
        (
            json!({"op": "edit_plan", "task": "a", "version": 0, "approach": ""}),
            at("version", "version", "invalid_value"),
        ),
        (
            json!({"op": "show", "task": "a", "version": 0}),
            at("version", "version", "invalid_value"),
        ),
        (
            propose(json!([{"id": "i"}]), Value::Null),
            at("items/0/title", "items", "missing"),
        ),
        // Ranges are the request's to name, by the domain's own limits.
        (
            propose(json!([]), role(json!({"members": 0}))),
            at("team/roles/0/members", "team", "invalid_value"),
        ),
        (
            propose(json!([]), role(json!({"members": 33}))),
            at("team/roles/0/members", "team", "invalid_value"),
        ),
        (
            propose(json!([]), role(json!({"turns": 0}))),
            at("team/roles/0/turns", "team", "invalid_value"),
        ),
        (
            propose(json!([]), json!({"roles": [], "budget_tokens": 0})),
            at("team/budget_tokens", "team", "invalid_value"),
        ),
        (
            propose(
                json!([]),
                json!({"roles": [], "budget_tokens": 1_u64 << 53}),
            ),
            at("team/budget_tokens", "team", "invalid_value"),
        ),
    ];
    for (value, want) in cases {
        assert_eq!(fault(value.clone()), want, "{value}");
    }
    // A role missing any required key is refused naming it: never a panic.
    let whole = json!({"name": "w", "model": "m", "effort": "low", "members": 1});
    for key in ["name", "model", "effort", "members"] {
        let mut missing = whole.clone();
        missing.as_object_mut().unwrap().remove(key);
        let value = propose(json!([]), json!({"roles": [missing]}));
        let path = format!("team/roles/0/{key}");
        assert_eq!(fault(value), at(&path, "team", "missing"), "{key}");
    }
}

/// #2481 M4c: an acceptance names the draft and the `updated` it read;
/// a discard may.
#[test]
fn an_acceptance_requires_the_updated_it_read_and_a_discard_may_give_one() {
    use crate::application::project_board::dto::{PlanAcceptance, PlanDiscard};
    let read = Timestamp::parse("2026-10-10T09:00:00Z").unwrap();
    assert_eq!(
        parse(&json!({"op": "accept_plan", "task": "a", "version": 2,
                      "expected_updated": "2026-10-10T09:00:00Z"})),
        Ok(BoardRequest::AcceptPlan(PlanAcceptance {
            task: slug("a"),
            version: 2,
            expected_updated: read.clone(),
        }))
    );
    let unread = refused(json!({"op": "accept_plan", "task": "a", "version": 2}));
    assert_eq!(
        (
            unread.field.as_str(),
            unread.fault.field.as_str(),
            unread.fault.fault.as_str()
        ),
        ("expected_updated", "expected_updated", "missing")
    );
    assert_eq!(
        parse(&json!({"op": "discard_plan", "task": "a", "version": 1})),
        Ok(BoardRequest::DiscardPlan(PlanDiscard {
            task: slug("a"),
            version: 1,
            expected_updated: None,
        }))
    );
    let zero = refused(json!({"op": "discard_plan", "task": "a", "version": 0}));
    assert_eq!(zero.field, "version");
}
