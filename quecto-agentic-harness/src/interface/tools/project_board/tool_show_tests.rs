//! The agent tool's `show` within its cap (#2481 M4a review round 1): a
//! plan, runs and versions at the board's limits come back within
//! [`MAX_SHOW_BYTES`], the task whole; what was left out is counted; a
//! show that fits is given as it is.
use serde_json::{Value, json};

use super::{MAX_SHOW_BYTES, capped};
use crate::application::project_board::dto::ShownTask;
use crate::application::project_board::operations::BoardOutcome;
use crate::application::project_board::ports::{ArtifactKey, BoardHead, QuarantinedFile};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::plan::{
    MAX_APPROACH_CHARS, MAX_PLAN_VERSION, Plan, PlanFields, PlanItem,
};
use crate::domain::project_board::entities::run::{MAX_RUN_NUMBER, Run, RunFields, RunItem};
use crate::domain::project_board::entities::task::{MAX_DESCRIPTION_CHARS, Task, TaskFields};
use crate::domain::project_board::entities::task_parts::{MAX_ITEMS, MAX_LINE_CHARS};
use crate::domain::project_board::entities::team::{Role, RoleLimits, Team};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{
    Effort, ItemState, PlanState, RunOutcome, TaskKind, TaskStatus,
};
use crate::interface::tools::project_board::answer::outcome;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn at() -> Timestamp {
    Timestamp::parse("2026-10-10T09:00:00Z").unwrap()
}

fn ada() -> Identity {
    Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    }
}

/// Task `a`, naming plan `accepted`, with `description`.
fn task(description: String, accepted: u32) -> Task {
    Task::new(TaskFields {
        id: slug("a"),
        title: "Task a".into(),
        kind: TaskKind::Task,
        description,
        status: TaskStatus::Ready,
        parent: None,
        depends_on: vec![],
        accepted_plan: Some(accepted),
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at(),
        updated: at(),
    })
    .unwrap()
}

/// Version `version` of task `a`'s plan in `state`, with `approach` and
/// `items`.
fn plan(version: u32, state: PlanState, approach: String, items: Vec<PlanItem>) -> Plan {
    let accepted = matches!(state, PlanState::Accepted | PlanState::Superseded);
    Plan::new(PlanFields {
        task: slug("a"),
        version,
        state,
        approach,
        items,
        team: Some(Team {
            roles: vec![Role {
                name: slug("worker"),
                model: "gpt-5.5".into(),
                effort: Effort::High,
                limits: RoleLimits {
                    members: 2,
                    turns: None,
                },
            }],
            budget: None,
            deadline: None,
            image: None,
        }),
        proposed_by: ada(),
        edited_by: None,
        accepted: accepted.then(|| (ada(), at())),
        created: at(),
        updated: at(),
    })
    .unwrap()
}

/// A plan at the board's limits, as JSON's widest: an approach of quotes
/// (each escaped), and every task item with its title and criteria as
/// long as the plan's text budget lets them be.
fn widest_plan(version: u32) -> Plan {
    let approach = "\"".repeat(MAX_APPROACH_CHARS);
    let items = (0..MAX_ITEMS)
        .map(|index| PlanItem {
            id: slug(&format!("item-{index}")),
            title: format!("{index}{}", "\"".repeat(100)),
            criteria: (0..3)
                .map(|n| format!("{n}{}", "\"".repeat(MAX_LINE_CHARS - 1)))
                .collect(),
            depends_on: vec![],
        })
        .collect();
    plan(version, PlanState::Accepted, approach, items)
}

/// Run `number` of plan `version`, ended, with `items` task items, each
/// with an owner and three evidence lines as long as they go.
fn run(number: u32, version: u32, items: usize) -> Run {
    Run::new(RunFields {
        task: slug("a"),
        number,
        plan_version: version,
        started: at(),
        ended: Some(at()),
        outcome: Some(RunOutcome::Failed),
        cost: Some(1),
        items: (0..items)
            .map(|index| RunItem {
                id: slug(&format!("item-{index}")),
                state: ItemState::Done,
                owner: Some(format!("{index}{}", "w".repeat(90))),
                evidence: (0..3)
                    .map(|n| format!("{n}{}", "\"".repeat(MAX_LINE_CHARS - 1)))
                    .collect(),
            })
            .collect(),
    })
    .unwrap()
}

/// A show at the board's limits: the widest plan accepted as the last of
/// every version, every run number with the latest at its widest, and
/// files set aside, each with the longest reason shown.
fn widest(description: String) -> ShownTask {
    let last = MAX_PLAN_VERSION;
    let mut plans: Vec<Plan> = (1..last)
        .map(|version| plan(version, PlanState::Superseded, "x".into(), vec![]))
        .collect();
    plans.push(widest_plan(last));
    let mut runs: Vec<Run> = (1..MAX_RUN_NUMBER).map(|n| run(n, last, 0)).collect();
    runs.push(run(MAX_RUN_NUMBER, last, MAX_ITEMS));
    let set_aside = (1..=200)
        .map(|n| QuarantinedFile {
            path: format!("runs/a.{n}.md"),
            key: Some(ArtifactKey::Run(slug("a"), n)),
            reason: "r".repeat(3_000),
            object: "0".repeat(40),
            carried: true,
        })
        .collect();
    ShownTask {
        head: BoardHead("0".repeat(40)),
        task: task(description, last),
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
    )
}

fn bytes(value: &Value) -> usize {
    value.to_string().len()
}

/// A count an answer gives, `0` when it gives none.
fn count(answer: &Value, key: &str) -> usize {
    answer[key].as_u64().map_or(0, |count| count as usize)
}

/// A cut answer shows the newest runs and plan versions, each in its
/// order and none skipped, no run without the latest, and the first
/// files set aside in theirs.
fn newest_and_first(cut: &Value) {
    let numbers = |list: &str, key: &str| -> Vec<u64> {
        let list = cut[list].as_array().unwrap();
        list.iter()
            .map(|entry| entry[key].as_u64().unwrap())
            .collect()
    };
    let runs = numbers("runs", "number");
    let latest = u64::from(MAX_RUN_NUMBER);
    let newest = (latest + 1 - runs.len() as u64..=latest).collect::<Vec<u64>>();
    assert_eq!(runs, newest, "the newest runs, the latest among them");
    let versions = numbers("plans", "version");
    let last = u64::from(MAX_PLAN_VERSION);
    let newest = (last + 1 - versions.len() as u64..=last).collect::<Vec<u64>>();
    assert_eq!(versions, newest, "the newest plan versions");
    let paths: Vec<String> = cut["quarantined"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap().to_owned())
        .collect();
    let first: Vec<String> = (1..=paths.len())
        .map(|n| format!("runs/a.{n}.md"))
        .collect();
    assert_eq!(paths, first, "the first files set aside");
}

/// What a list's entries and their count of what was left out add up to.
fn whole(answer: &Value, list: &str, more: &str) -> usize {
    answer[list].as_array().unwrap().len() + answer[more].as_u64().unwrap_or(0) as usize
}

#[test]
fn a_show_at_the_boards_limits_fits_its_cap() {
    let uncut = answered(widest("Why.".into()));
    // Measured: the whole answer the CLI gives, far past any context.
    assert!(bytes(&uncut) > 1_500_000, "{}", bytes(&uncut));
    let cut = capped(uncut.clone());
    assert!(bytes(&cut) <= MAX_SHOW_BYTES, "{}", bytes(&cut));
    assert_eq!(cut["task"], uncut["task"], "the task is whole");
    // The plan's fields stay; its approach and items are cut and counted.
    let (plan, all) = (&cut["plan"], &uncut["plan"]);
    for field in [
        "task",
        "version",
        "state",
        "team",
        "proposed_by",
        "accepted_by",
    ] {
        assert_eq!(plan[field], all[field], "{field}");
    }
    let approach = plan["approach"].as_str().unwrap();
    assert!(all["approach"].as_str().unwrap().starts_with(approach));
    assert_eq!(cut["plan_truncated"], json!(true));
    let shown_chars = approach.chars().count();
    assert_eq!(
        shown_chars + count(&cut, "approach_more"),
        MAX_APPROACH_CHARS
    );
    assert_eq!(count(&cut, "approach_next"), shown_chars);
    let items = plan["items"].as_array().unwrap();
    assert_eq!(items.len() + count(&cut, "items_more"), MAX_ITEMS);
    assert_eq!(count(&cut, "items_next"), items.len());
    assert_eq!(items[..], all["items"].as_array().unwrap()[..items.len()]);
    // The newest runs and versions, in their order, counted.
    let runs = cut["runs"].as_array().unwrap();
    assert_eq!(runs.last().unwrap()["number"], json!(MAX_RUN_NUMBER));
    assert_eq!(whole(&cut, "runs", "runs_more"), MAX_RUN_NUMBER as usize);
    let latest_items = runs.last().unwrap()["items"].as_array().unwrap().len();
    assert_eq!(
        latest_items + cut["latest_run_items_more"].as_u64().unwrap_or(0) as usize,
        MAX_ITEMS
    );
    let numbers: Vec<u64> = runs.iter().map(|r| r["number"].as_u64().unwrap()).collect();
    assert!(numbers.windows(2).all(|pair| pair[0] + 1 == pair[1]));
    assert_eq!(
        whole(&cut, "plans", "plans_more"),
        MAX_PLAN_VERSION as usize
    );
    assert_eq!(whole(&cut, "quarantined", "quarantined_more"), 200);
    newest_and_first(&cut);
    assert_eq!(cut["may_be_incomplete"], json!(true));
}

/// A task is never cut: with its longest description, which takes the
/// room alone, the rest is counted and not shown.
#[test]
fn the_task_is_whole_and_the_rest_counted() {
    let description = "\"".repeat(MAX_DESCRIPTION_CHARS);
    let cut = capped(answered(widest(description)));
    assert_eq!(
        cut["task"]["description"].as_str().unwrap().chars().count(),
        MAX_DESCRIPTION_CHARS
    );
    assert_eq!(cut["plan"], json!(null));
    assert_eq!(cut["plan_truncated"], json!(true));
    assert_eq!(
        (count(&cut, "approach_more"), count(&cut, "approach_next")),
        (MAX_APPROACH_CHARS, 0)
    );
    assert_eq!(
        (count(&cut, "items_more"), count(&cut, "items_next")),
        (MAX_ITEMS, 0)
    );
    assert_eq!(cut["runs"], json!([]));
    assert_eq!(cut["runs_more"], json!(MAX_RUN_NUMBER));
    assert_eq!(cut["plans_more"], json!(MAX_PLAN_VERSION));
    assert_eq!(cut["quarantined_more"], json!(200));
    // The task and the answer's own fields, nothing more.
    let task = bytes(&cut["task"]);
    assert!(bytes(&cut) - task < 1_024, "{}", bytes(&cut) - task);
}

/// A show that fits is given exactly as the board answered it.
#[test]
fn a_show_that_fits_is_given_as_it_is() {
    let small = ShownTask {
        head: BoardHead("0".repeat(40)),
        task: task("Why.".into(), 1),
        plans: vec![plan(1, PlanState::Accepted, "Build it.".into(), vec![])],
        runs: vec![run(1, 1, 2), run(2, 1, 2)],
        set_aside: vec![],
    };
    let answer = answered(small);
    assert_eq!(capped(answer.clone()), answer);
}

/// A plan that fits stays whole, even when its runs do not all fit.
#[test]
fn a_plan_that_fits_stays_whole_before_the_runs() {
    let mut shown = widest("Why.".into());
    let fits = plan(
        MAX_PLAN_VERSION,
        PlanState::Accepted,
        "Build it.".into(),
        vec![],
    );
    *shown.plans.last_mut().unwrap() = fits;
    let uncut = answered(shown);
    let cut = capped(uncut.clone());
    assert_eq!(cut["plan"], uncut["plan"]);
    assert_eq!(cut.get("plan_truncated"), None);
    assert!(cut["runs_more"].as_u64().unwrap() > 0);
    newest_and_first(&cut);
}

/// The tool caps every answered show it gives; a refused one, and any
/// other op's answer but a listing, pass as they are.
#[test]
fn the_tool_caps_every_answered_show() {
    let uncut = answered(widest("Why.".into()));
    assert!(bytes(&super::super::summarised(uncut)) <= MAX_SHOW_BYTES);
    let refused = json!({"ok": false, "op": "show", "error": {"code": "not_found"}});
    assert_eq!(super::super::summarised(refused.clone()), refused);
}

/// A show of task `a` with its accepted plan's approach `approach` and
/// two task items, and nothing else.
fn plan_only(approach: String) -> Value {
    let item = |id: &str| PlanItem {
        id: slug(id),
        title: format!("Build {id}"),
        criteria: vec!["it is built".into()],
        depends_on: vec![],
    };
    let plan = plan(1, PlanState::Accepted, approach, vec![item("x"), item("y")]);
    answered(ShownTask {
        head: BoardHead("0".repeat(40)),
        task: task("Why.".into(), 1),
        plans: vec![plan],
        runs: vec![],
        set_aside: vec![],
    })
}

/// A plan-only show `over` bytes past the cap (or short of it, below 0).
fn plan_only_over(over: isize) -> Value {
    let base = bytes(&plan_only("a".into()));
    let chars = (MAX_SHOW_BYTES as isize - base as isize + 1 + over) as usize;
    let answer = plan_only("a".repeat(chars));
    assert_eq!(bytes(&answer) as isize, MAX_SHOW_BYTES as isize + over);
    answer
}

/// An answer exactly at the cap is given as it is; one byte over is cut
/// to fit, the approach losing about that byte and the room the counts
/// take, its items whole.
#[test]
fn the_cap_cuts_only_past_it_and_only_what_it_must() {
    let at = plan_only_over(0);
    assert_eq!(capped(at.clone()), at);
    for over in [1, 932] {
        let cut = capped(plan_only_over(over));
        assert!(bytes(&cut) <= MAX_SHOW_BYTES, "{over}: {}", bytes(&cut));
        let more = count(&cut, "approach_more");
        assert!(
            more >= over as usize && more < over as usize + 400,
            "{over}: {more}"
        );
        assert_eq!(cut["plan"]["items"].as_array().unwrap().len(), 2);
        assert_eq!(cut.get("items_more"), None);
    }
}

/// `show` given where to read on pages through the accepted plan: from
/// each cut page's `approach_next` and `items_next` on, the pages hold the
/// whole approach and every task item, each page within the cap.
#[test]
fn pages_read_on_through_the_whole_plan() {
    use crate::application::project_board::operations::PlanPage;
    let shown = widest("Why.".into());
    let full = widest_plan(MAX_PLAN_VERSION);
    let (mut approach, mut items) = (String::new(), Vec::new());
    let mut next = (Some(0), Some(0));
    let mut pages = 0;
    while let Some(page) = PlanPage::of(next.0, next.1) {
        pages += 1;
        assert!(pages < 100, "pages always read on");
        let answer = outcome(
            "show",
            &BoardOutcome::Shown(
                shown.clone(),
                crate::application::project_board::operations::PlanView {
                    version: None,
                    page: Some(page),
                },
            ),
            None,
        );
        let cut = capped(answer);
        assert!(bytes(&cut) <= MAX_SHOW_BYTES, "{}", bytes(&cut));
        assert_eq!(cut["task"], json!("a"));
        let plan = &cut["plan"];
        if let Some(text) = plan["approach"].as_str() {
            assert_eq!(plan["approach_from"], json!(approach.chars().count()));
            approach.push_str(text);
        }
        if let Some(page_items) = plan["items"].as_array() {
            assert_eq!(plan["items_from"], json!(items.len()));
            items.extend(page_items.iter().map(|item| item["id"].clone()));
        }
        let at = |key: &str| cut[key].as_u64().map(|at| at as usize);
        let read_on = (at("approach_next"), at("items_next"));
        moves_forward(&cut, page);
        next = read_on;
    }
    assert!(pages > 1, "a plan at the limits takes more than one page");
    assert_eq!(approach, full.fields().approach);
    let ids: Vec<Value> = full
        .fields()
        .items
        .iter()
        .map(|item| json!(item.id.as_str()))
        .collect();
    assert_eq!(items, ids);
}

/// Once a whole character as read fits, a cut never leaves one half
/// shown: never before a combining mark, a variation selector or an emoji
/// modifier, never after a zero-width joiner. (Below that, the page still
/// moves on: `a_cut_keeps_clusters_whole_where_it_can_and_always_moves_on`.)
#[test]
fn an_approach_is_cut_where_no_character_joins_the_next() {
    use super::prefix_within;
    let accented = "e\u{301}".repeat(10);
    let first = "e\u{301}".len();
    for room in first..accented.len() {
        let cut = prefix_within(&accented, room);
        assert!(cut.chars().count() % 2 == 0, "{room}: {cut:?}");
    }
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}x".repeat(4);
    let first = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}".len();
    for room in first..family.len() {
        let cut = prefix_within(&family, room);
        assert!(!cut.ends_with('\u{200D}'), "{room}: {cut:?}");
        let rest = &family[cut.len()..];
        assert!(!rest.starts_with('\u{200D}'), "{room}: {cut:?}");
        assert!(cut.len() <= room);
    }
    let waved = "\u{1F44B}\u{1F3FD}\u{2764}\u{FE0F}".repeat(3);
    for room in "\u{1F44B}\u{1F3FD}".len()..waved.len() {
        let rest = &waved[prefix_within(&waved, room).len()..];
        assert!(
            !rest.starts_with(['\u{1F3FD}', '\u{FE0F}']),
            "{room}: {rest:?}"
        );
    }
}

/// Every cut page reads on past where it began: `approach_next` past
/// `approach_from`, `items_next` past `items_from`.
fn moves_forward(cut: &Value, page: crate::application::project_board::operations::PlanPage) {
    let at = |key: &str| cut[key].as_u64().map(|at| at as usize);
    if let (Some(from), Some(next)) = (page.approach_from(), at("approach_next")) {
        assert!(next > from, "approach_next {next} past {from}");
    }
    if let (Some(from), Some(next)) = (page.items_from(), at("items_next")) {
        assert!(next > from, "items_next {next} past {from}");
    }
}

/// Review round 3: an approach with no safe cut (one letter and forty
/// thousand combining marks) is still read on page by page: a page cuts
/// inside the cluster rather than read nothing, and the pages hold it
/// whole.
#[test]
fn a_page_with_no_safe_cut_still_reads_on() {
    use crate::application::project_board::operations::PlanPage;
    let full = format!("e{}", "\u{301}".repeat(40_000));
    let page = |from: usize| {
        let approach: String = full.chars().skip(from).collect();
        json!({
            "ok": true, "op": "show", "head": "0".repeat(40), "task": "a",
            "plan": {"version": 1, "approach_from": from, "approach": approach},
            "may_be_incomplete": false,
        })
    };
    let (mut read, mut from, mut pages) = (String::new(), Some(0), 0);
    while let Some(at) = from {
        pages += 1;
        assert!(pages < 20, "pages always read on");
        let cut = capped(page(at));
        assert!(bytes(&cut) <= MAX_SHOW_BYTES, "{}", bytes(&cut));
        read.push_str(cut["plan"]["approach"].as_str().unwrap());
        moves_forward(&cut, PlanPage::of(Some(at), None).unwrap());
        from = cut["approach_next"].as_u64().map(|next| next as usize);
    }
    assert!(pages > 1);
    assert_eq!(read, full);
}

/// A cut that can keep a cluster whole does; one that cannot keeps every
/// character that fits rather than none.
#[test]
fn a_cut_keeps_clusters_whole_where_it_can_and_always_moves_on() {
    use super::prefix_within;
    let marks = format!("e{}", "\u{301}".repeat(100));
    for room in 1..marks.len() {
        assert!(!prefix_within(&marks, room).is_empty(), "{room}");
    }
    assert_eq!(prefix_within(&marks, 0), "");
    // Flags are pairs of regional indicators: never cut between a pair
    // once one whole flag fits.
    let flags = "\u{1F1EC}\u{1F1E7}\u{1F1EB}\u{1F1F7}".repeat(3);
    for room in 8..flags.len() {
        let kept = prefix_within(&flags, room).chars().count();
        assert!(kept % 2 == 0, "{room}: {kept}");
    }
}

/// Review round 4: an end_run's answer says whether its front end looked
/// for the run's swarm still running, and, when it could not, what that
/// leaves its caller; a holder's end says its items were recorded as todo.
#[test]
fn an_end_runs_answer_says_whether_its_swarm_was_looked_for() {
    use crate::application::project_board::dto::{EndedRun, LivenessCheck};
    let answered = |liveness| {
        crate::interface::tools::project_board::answer::outcome(
            "end_run",
            &BoardOutcome::RunEnded(EndedRun {
                run: run(1, 1, 0),
                liveness,
            }),
            None,
        )
    };
    let unchecked = answered(LivenessCheck::NotChecked);
    assert_eq!(unchecked["liveness"], json!("not_checked"));
    assert_eq!(
        unchecked["liveness_note"],
        json!(
            "this end was not checked against running swarms: if the run's coordinator still \
             runs, stop it"
        )
    );
    assert_eq!(
        unchecked["items_recorded"],
        json!("items recorded as todo: no swarm report")
    );
    let checked = answered(LivenessCheck::Checked);
    assert_eq!(checked["liveness"], json!("checked"));
    assert_eq!(checked.get("liveness_note"), None);
}
