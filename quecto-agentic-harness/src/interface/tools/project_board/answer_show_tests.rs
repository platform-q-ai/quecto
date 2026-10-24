//! `show`'s answer (#2481 M4a): the task, its accepted plan whole, each
//! plan version in brief, its runs (the latest whole), and whether a plan
//! or run file is set aside; and every set-aside file named by its kind.
use super::*;
use crate::application::project_board::dto::ShownTask;
use crate::domain::project_board::entities::plan::{Plan, PlanFields, PlanItem};
use crate::domain::project_board::entities::run::{Run, RunFields, RunItem};
use crate::domain::project_board::entities::task::TaskFields;
use crate::domain::project_board::entities::team::{Role, RoleLimits, Team};
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{
    Effort, ItemState, PlanState, RunOutcome, TaskKind, TaskStatus,
};

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn at(time: &str) -> Timestamp {
    Timestamp::parse(&format!("2026-10-10T{time}Z")).unwrap()
}

fn ada() -> Identity {
    Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    }
}

/// Task `a`, naming plan 2 as accepted.
fn task() -> Task {
    Task::new(TaskFields {
        id: slug("a"),
        title: "Task a".into(),
        kind: TaskKind::Task,
        description: String::new(),
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        accepted_plan: Some(2),
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at("08:00:00"),
        updated: at("08:30:00"),
        description_form: Default::default(),
    })
    .unwrap()
}

/// Version `version` of task `a`'s plan in `state`: one item, and on
/// version 2 a team.
fn plan(version: u32, state: PlanState) -> Plan {
    let team = (version == 2).then(|| Team {
        roles: vec![Role {
            name: slug("worker"),
            model: "gpt-5.5".into(),
            effort: Effort::High,
            limits: RoleLimits {
                members: 2,
                turns: None,
            },
        }],
        deadline_seconds: Some(3_600),
        image: None,
    });
    Plan::new(PlanFields {
        task: slug("a"),
        version,
        state,
        approach: "Spike, then build.".into(),
        items: vec![PlanItem {
            id: slug("build"),
            title: "Build it".into(),
            criteria: vec!["it is built".into()],
            depends_on: vec![],
        }],
        team,
        handoff: None,
        proposed_by: ada(),
        edited_by: None,
        accepted: Some((ada(), at("08:30:00"))),
        created: at("08:00:00"),
        updated: at("08:30:00"),
    })
    .unwrap()
}

/// Run `number` of task `a`'s plan `version`, ended with `outcome`, its
/// one item where `state`, with `evidence`.
fn run(number: u32, version: u32, outcome: RunOutcome, evidence: &[&str]) -> Run {
    Run::new(RunFields {
        task: slug("a"),
        number,
        plan_version: version,
        started: at("09:00:00"),
        ended: Some(at("10:00:00")),
        outcome: Some(outcome),
        cost: Some(500),
        items: vec![RunItem {
            id: slug("build"),
            state: ItemState::Done,
            owner: Some("w-1".into()),
            evidence: evidence.iter().map(|line| line.to_string()).collect(),
        }],
    })
    .unwrap()
}

fn shown(plans: Vec<Plan>, runs: Vec<Run>, set_aside: Vec<QuarantinedFile>) -> ShownTask {
    ShownTask {
        head: BoardHead("0".repeat(40)),
        task: task(),
        plans,
        runs,
        set_aside,
    }
}

fn answered(shown: ShownTask) -> Value {
    outcome(
        "show",
        &BoardOutcome::Shown(
            shown,
            crate::application::project_board::operations::PlanView::default(),
        ),
        None,
        None,
    )
}

/// The golden answer: every field in its order.
#[test]
fn show_answers_the_accepted_plan_each_version_and_the_runs() {
    let answer = answered(shown(
        vec![plan(1, PlanState::Superseded), plan(2, PlanState::Accepted)],
        vec![
            run(1, 1, RunOutcome::Failed, &["tried"]),
            run(
                2,
                2,
                RunOutcome::Succeeded,
                &["[PR](https://example.com/p/1) at abc1234"],
            ),
        ],
        vec![],
    ));
    let identity = json!({"name": "Ada", "email": "ada@example.com"});
    let golden = json!({
        "ok": true,
        "op": "show",
        "head": "0".repeat(40),
        "task": task_json(&task()),
        "plan": {
            "task": "a",
            "version": 2,
            "state": "accepted",
            "approach": "Spike, then build.",
            "items": [{
                "id": "build",
                "title": "Build it",
                "criteria": ["it is built"],
                "depends_on": [],
            }],
            "team": {
                "roles": [{
                    "name": "worker",
                    "model": "gpt-5.5",
                    "effort": "high",
                    "members": 2,
                    "turns": null,
                }],
                "deadline_seconds": 3_600,
                "image": null,
            },
            "proposed_by": identity,
            "edited_by": null,
            "accepted_by": identity,
            "accepted_at": "2026-10-10T08:30:00Z",
            "created": "2026-10-10T08:00:00Z",
            "updated": "2026-10-10T08:30:00Z",
        },
        "plans": [
            {"version": 1, "state": "superseded", "proposed_by": identity, "updated": "2026-10-10T08:30:00Z"},
            {"version": 2, "state": "accepted", "proposed_by": identity, "updated": "2026-10-10T08:30:00Z"},
        ],
        "runs": [
            {
                "number": 1,
                "plan_version": 1,
                "state": "failed",
                "started": "2026-10-10T09:00:00Z",
                "ended": "2026-10-10T10:00:00Z",
                "cost_tokens": 500,
            },
            {
                "task": "a",
                "number": 2,
                "plan_version": 2,
                "state": "succeeded",
                "started": "2026-10-10T09:00:00Z",
                "ended": "2026-10-10T10:00:00Z",
                "cost_tokens": 500,
                "items": [{
                    "id": "build",
                    "state": "done",
                    "owner": "w-1",
                    "evidence": ["[PR](https://example.com/p/1) at abc1234"],
                }],
            },
        ],
        "may_be_incomplete": false,
        "quarantined": [],
    });
    assert_eq!(answer, golden);
    // In this order, as a reader sees it.
    assert_eq!(answer.to_string(), golden.to_string());
}

/// A task with no plan and no run answers them empty; a running run is
/// `running` with no end, cost or items yet.
#[test]
fn show_answers_no_plan_and_a_running_run() {
    let empty = answered(shown(vec![], vec![], vec![]));
    assert_eq!(
        (&empty["plan"], &empty["plans"], &empty["runs"]),
        (&json!(null), &json!([]), &json!([]))
    );
    let going = Run::new(RunFields {
        ended: None,
        outcome: None,
        cost: None,
        items: vec![],
        ..run(1, 2, RunOutcome::Failed, &[]).into_fields()
    })
    .unwrap();
    let answer = answered(shown(
        vec![plan(2, PlanState::Accepted)],
        vec![going],
        vec![],
    ));
    let run = &answer["runs"][0];
    assert_eq!(
        (
            &run["state"],
            &run["ended"],
            &run["cost_tokens"],
            &run["items"]
        ),
        (&json!("running"), &json!(null), &json!(null), &json!([]))
    );
}

/// A run's owner and evidence came from a swarm's report: credentials in
/// either are redacted, as every text from outside is.
#[test]
fn a_runs_owner_and_evidence_are_redacted() {
    let secret = "sk-abcdefghijklmnopqrstuvwxyz";
    let leaky = run(
        1,
        1,
        RunOutcome::Stopped,
        &[&format!("key {secret}"), "kept"],
    );
    let mut fields = leaky.into_fields();
    fields.items[0].owner = Some(format!("w {secret}"));
    let answer = answered(shown(vec![], vec![Run::new(fields).unwrap()], vec![]));
    let item = &answer["runs"][0]["items"][0];
    let (owner, evidence) = (item["owner"].to_string(), item["evidence"].to_string());
    assert!(!owner.contains(secret), "{owner}");
    assert!(owner.starts_with("\"w "), "the rest is kept: {owner}");
    assert!(!evidence.contains(secret), "{evidence}");
    assert_eq!(item["evidence"][1], json!("kept"));
}

/// A plan or run file set aside leaves the answer `may_be_incomplete` and
/// names it under `quarantined` by its kind, task and number, as
/// `artifact_set_aside` does; the task's plan is then unknown, so none is
/// shown.
#[test]
fn a_set_aside_plan_is_named_and_the_answer_may_be_incomplete() {
    let file = QuarantinedFile {
        path: "plans/a.2.md".into(),
        key: Some(ArtifactKey::Plan(slug("a"), 2)),
        reason: "malformed".into(),
        object: "b".repeat(40),
        carried: true,
    };
    let answer = answered(shown(
        vec![plan(1, PlanState::Superseded)],
        vec![],
        vec![file],
    ));
    assert_eq!(answer["may_be_incomplete"], json!(true));
    assert_eq!(answer["plan"], json!(null));
    assert_eq!(
        answer["quarantined"],
        json!([{
            "path": "plans/a.2.md",
            "id": null,
            "kind": "plan",
            "task": "a",
            "number": 2,
            "reason": "malformed",
            "object": "b".repeat(40),
            "copies": ["b".repeat(40)],
        }])
    );
}

/// Every kind's set-aside file is named by its key in its own folder; a
/// discussion by its id.
#[test]
fn a_set_aside_file_of_every_kind_is_named_by_its_key() {
    let a = slug("a");
    for (key, path, named) in [
        (
            ArtifactKey::Task(a.clone()),
            "tasks/a.md",
            json!({"id": "a"}),
        ),
        (
            ArtifactKey::Run(a.clone(), 3),
            "runs/a.3.md",
            json!({"id": null, "kind": "run", "task": "a", "number": 3}),
        ),
        (
            ArtifactKey::Change(a.clone(), 1),
            "changes/a.1.md",
            json!({"id": null, "kind": "change", "task": "a", "number": 1}),
        ),
        (
            ArtifactKey::Review(a.clone(), 1),
            "reviews/a.1.md",
            json!({"id": null, "kind": "review", "task": "a", "number": 1}),
        ),
        (
            ArtifactKey::Discussion(slug("d")),
            "discussions/d.md",
            json!({"id": null, "kind": "discussion", "discussion": "d"}),
        ),
    ] {
        let entry = quarantined_json(&QuarantinedFile {
            path: path.into(),
            key: Some(key),
            reason: "malformed".into(),
            object: "a".repeat(40),
            carried: true,
        });
        for (field, value) in named.as_object().unwrap() {
            assert_eq!(&entry[field], value, "{path}: {field}");
        }
        assert_eq!(entry["path"], json!(path));
    }
}

/// A file set aside under one kind's key but shown in another kind's
/// folder stops the answer: a store names each in its own.
#[test]
#[should_panic(expected = "shows every path it sets aside sanitised")]
fn a_set_aside_file_outside_its_kinds_folder_is_never_shown() {
    let _ = quarantined_json(&QuarantinedFile {
        path: "plans/a.1.md".into(),
        key: Some(ArtifactKey::Run(slug("a"), 1)),
        reason: "malformed".into(),
        object: "a".repeat(40),
        carried: true,
    });
}

/// A `show` given a page answers the task's id and only the parts of its
/// accepted plan the page names, from where it names; a part not named is
/// left out, an offset past the end reads nothing, and a task with no plan
/// to page through answers `null`.
#[test]
fn a_page_answers_only_the_parts_it_names_from_where_it_names() {
    use crate::application::project_board::operations::PlanPage;
    let paged = |plans: Vec<Plan>, page: Option<PlanPage>| {
        outcome(
            "show",
            &BoardOutcome::Shown(
                shown(plans, vec![], vec![]),
                crate::application::project_board::operations::PlanView {
                    version: None,
                    page,
                },
            ),
            None,
            None,
        )
    };
    let accepted = || vec![plan(1, PlanState::Superseded), plan(2, PlanState::Accepted)];
    let both = paged(accepted(), PlanPage::of(Some(7), Some(0)));
    assert_eq!(
        both,
        json!({
            "ok": true, "op": "show", "head": "0".repeat(40), "task": "a",
            "plan": {
                "version": 2,
                "updated": "2026-10-10T08:30:00Z",
                "approach_from": 7,
                "approach": "then build.",
                "items_from": 0,
                "items": [{"id": "build", "title": "Build it", "criteria": ["it is built"], "depends_on": []}],
            },
            "may_be_incomplete": false,
        })
    );
    let items_only = paged(accepted(), PlanPage::of(None, Some(1)));
    assert_eq!(
        items_only["plan"],
        json!({"version": 2, "updated": "2026-10-10T08:30:00Z", "items_from": 1, "items": []})
    );
    let past = paged(accepted(), PlanPage::of(Some(9_999), None));
    assert_eq!(
        past["plan"],
        json!({"version": 2, "updated": "2026-10-10T08:30:00Z", "approach_from": 9_999, "approach": ""})
    );
    let none = paged(
        vec![plan(1, PlanState::Superseded)],
        PlanPage::of(Some(0), None),
    );
    assert_eq!(none["plan"], json!(null));
}
