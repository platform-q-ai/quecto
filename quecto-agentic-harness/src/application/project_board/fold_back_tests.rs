use super::*;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::plan::{Plan, PlanFields, PlanItem};
use crate::domain::project_board::entities::task_parts::MAX_BUDGET_TOKENS;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::PlanState;
use crate::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

fn at() -> Timestamp {
    Timestamp::parse("2026-10-10T08:30:00Z").unwrap()
}

fn ada() -> Identity {
    Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    }
}

/// Accepted version 1 of `board-store`'s plan: task items `api`, `store`
/// and `docs`, titled `Build the <id>`.
pub fn plan() -> Plan {
    let item = |id: &str| PlanItem {
        id: slug(id),
        title: format!("Build the {id}"),
        criteria: vec![format!("the {id} is built")],
        depends_on: vec![],
    };
    Plan::new(PlanFields {
        task: slug("board-store"),
        version: 1,
        state: PlanState::Accepted,
        approach: "Spike, then build.".into(),
        items: vec![item("api"), item("store"), item("docs")],
        team: None,
        proposed_by: ada(),
        edited_by: None,
        accepted: Some((ada(), at())),
        created: at(),
        updated: at(),
    })
    .unwrap()
}

/// A swarm item titled `title`, in `state`, held by `owner`.
pub fn swarm_item(
    title: &str,
    state: SwarmItemState,
    owner: Option<&str>,
    evidence: &[(&str, &str)],
) -> SwarmItemView {
    SwarmItemView {
        title: title.into(),
        state,
        owner: owner.map(Into::into),
        evidence: evidence
            .iter()
            .map(|(artifact, revision)| SwarmEvidence {
                artifact: (*artifact).into(),
                revision: (*revision).into(),
            })
            .collect(),
    }
}

fn view(items: Vec<SwarmItemView>) -> SwarmRunView {
    SwarmRunView {
        end: SwarmEnd::Succeeded,
        items,
        observed_tokens: 1234,
    }
}

fn folded(items: Vec<SwarmItemView>) -> FoldBackItems {
    run_items(&plan(), &view(items), &MarkdownTextCells)
}

#[test]
fn every_terminal_swarm_status_maps_to_its_outcome_and_no_other_ends() {
    let rows = [
        ("succeeded", Some(RunOutcome::Succeeded)),
        ("failed", Some(RunOutcome::Failed)),
        ("budget-exhausted", Some(RunOutcome::Failed)),
        ("blocked", Some(RunOutcome::Stopped)),
        ("cancelled", Some(RunOutcome::Stopped)),
        ("setup", None),
        ("running", None),
        ("paused", None),
        ("SUCCEEDED", None),
        ("done", None),
    ];
    for (status, outcome) in rows {
        let end = SwarmEnd::of(&RunState::new(status));
        assert_eq!(end.map(SwarmEnd::outcome), outcome, "{status}");
    }
    assert_eq!(SwarmEnd::ALL.len(), 5);
}

#[test]
fn every_swarm_item_state_maps_to_its_task_item_state_and_no_other_is_read() {
    let rows = [
        ("ready", Some(ItemState::Todo)),
        ("claimed", Some(ItemState::Doing)),
        ("submitted", Some(ItemState::Review)),
        ("completed", Some(ItemState::Done)),
        ("blocked", Some(ItemState::Blocked)),
        ("done", None),
        ("Ready", None),
    ];
    for (state, item) in rows {
        let read = SwarmItemState::of(&TaskState::new(state));
        assert_eq!(read.map(SwarmItemState::item_state), item, "{state}");
    }
}

#[test]
fn items_match_by_title_key_in_plan_order_and_the_rest_are_counted() {
    let items = folded(vec![
        swarm_item(
            "store: Build the store",
            SwarmItemState::Claimed,
            Some("worker-2"),
            &[],
        ),
        swarm_item(
            "api: Build the api",
            SwarmItemState::Completed,
            Some("worker-1"),
            &[],
        ),
        // A second item under one key, and one no plan item names.
        swarm_item("api: Build the api", SwarmItemState::Ready, None, &[]),
        swarm_item("lint: Lint it", SwarmItemState::Completed, None, &[]),
        // Close to a key, but not it.
        swarm_item("docs: build the docs", SwarmItemState::Completed, None, &[]),
    ]);
    let states: Vec<(&str, ItemState, Option<&str>)> = items
        .items
        .iter()
        .map(|item| (item.id.as_str(), item.state, item.owner.as_deref()))
        .collect();
    assert_eq!(
        states,
        [
            ("api", ItemState::Done, Some("worker-1")),
            ("store", ItemState::Doing, Some("worker-2")),
            ("docs", ItemState::Todo, None),
        ]
    );
    assert_eq!((items.unmatched, items.extra), (1, 3));
    assert_eq!(
        items.counts(),
        ItemCounts {
            todo: 1,
            doing: 1,
            review: 0,
            done: 1,
            blocked: 0
        }
    );
}

#[test]
fn a_swarm_that_seeded_nothing_leaves_every_task_item_to_do() {
    let items = folded(vec![]);
    assert!(items.items.iter().all(|item| item.state == ItemState::Todo
        && item.owner.is_none()
        && item.evidence.is_empty()));
    assert_eq!((items.items.len(), items.unmatched, items.extra), (3, 3, 0));
}

#[test]
fn evidence_is_artifact_at_revision_with_bare_urls_linked() {
    let items = folded(vec![swarm_item(
        "api: Build the api",
        SwarmItemState::Completed,
        Some("worker-1"),
        &[
            ("src/api.rs", "4f2a9c1"),
            ("https://github.com/o/r/pull/7", "4f2a9c1"),
            ("PR https://github.com/o/r/pull/8 merged", "main"),
        ],
    )]);
    assert_eq!(
        items.items[0].evidence,
        [
            "src/api.rs at 4f2a9c1",
            "[link](https://github.com/o/r/pull/7) at 4f2a9c1",
            "PR [link](https://github.com/o/r/pull/8) merged at main",
        ]
    );
    assert_eq!((items.evidence, items.dropped), (3, 0));
}

#[test]
fn evidence_the_codec_refuses_is_dropped_and_counted_never_the_fold_back() {
    let items = folded(vec![swarm_item(
        "api: Build the api",
        SwarmItemState::Completed,
        None,
        &[
            ("costs $5", "4f2a9c1"),
            ("mailed ada@example.com", "4f2a9c1"),
            ("http://example.com/x", "4f2a9c1"),
            ("www.example.com", "4f2a9c1"),
            ("src/api.rs", "4f2a9c1"),
        ],
    )]);
    assert_eq!(items.items[0].evidence, ["src/api.rs at 4f2a9c1"]);
    assert_eq!((items.evidence, items.dropped), (1, 4));
}

#[test]
fn evidence_is_normalised_and_kept_within_the_schema() {
    let long = "x".repeat(MAX_LINE_CHARS);
    let many: Vec<(String, String)> = (0..MAX_PER_ITEM + 2)
        .map(|n| (format!("file{n}.rs"), "abc".to_string()))
        .collect();
    let mut evidence: Vec<(&str, &str)> = vec![
        ("src/api.rs\u{a0}\t", "  4f2a9c1"),
        // The same line again, once normalised.
        ("src/api.rs", "4f2a9c1"),
        ("", "4f2a9c1"),
        ("src/api.rs", " "),
        (long.as_str(), "4f2a9c1"),
        ("two\nlines", "4f2a9c1"),
    ];
    evidence.extend(many.iter().map(|(a, r)| (a.as_str(), r.as_str())));
    let items = folded(vec![swarm_item(
        "api: Build the api",
        SwarmItemState::Completed,
        None,
        &evidence,
    )]);
    let kept = &items.items[0].evidence;
    assert_eq!(kept.len(), MAX_PER_ITEM);
    assert_eq!(kept[0], "src/api.rs at 4f2a9c1");
    assert_eq!(kept[1], "file0.rs at abc");
    // Duplicate, two empty parts, too long, two lines, and the three past
    // the per-item cap.
    assert_eq!(items.dropped, 8);
    assert_eq!(items.evidence, MAX_PER_ITEM);
}

#[test]
fn evidence_stops_at_the_runs_text_budget() {
    let line = "y".repeat(MAX_LINE_CHARS - " at abc".len());
    let per_item: Vec<(&str, &str)> = (0..MAX_PER_ITEM).map(|_| (line.as_str(), "abc")).collect();
    // Distinct lines per item: the budget, not distinctness, is in play.
    let titles: Vec<String> = (0..40).map(|n| format!("i{n}: Item {n}")).collect();
    let mut fields = plan().into_fields();
    fields.items = (0..40)
        .map(|n| PlanItem {
            id: slug(&format!("i{n}")),
            title: format!("Item {n}"),
            criteria: vec![],
            depends_on: vec![],
        })
        .collect();
    let plan = Plan::new(fields).unwrap();
    let items: Vec<SwarmItemView> = titles
        .iter()
        .map(|title| {
            let mut item = swarm_item(title, SwarmItemState::Completed, None, &[]);
            item.evidence = per_item
                .iter()
                .enumerate()
                .map(|(n, (artifact, revision))| SwarmEvidence {
                    artifact: format!("{n:02}{}", &artifact[2..]),
                    revision: (*revision).into(),
                })
                .collect();
            item
        })
        .collect();
    let folded = run_items(&plan, &view(items), &MarkdownTextCells);
    let bytes: usize = folded
        .items
        .iter()
        .flat_map(|item| &item.evidence)
        .map(String::len)
        .sum();
    assert!(bytes <= MAX_RUN_TEXT_BYTES, "{bytes}");
    assert!(bytes + MAX_LINE_CHARS > MAX_RUN_TEXT_BYTES, "{bytes}");
    assert_eq!(folded.evidence + folded.dropped, 40 * MAX_PER_ITEM);
    assert!(folded.dropped > 0);
}

#[test]
fn an_owner_is_the_member_id_capped_and_one_the_board_cannot_hold_is_none() {
    let long = "w".repeat(MAX_OWNER_CHARS + 20);
    let items = folded(vec![
        swarm_item(
            "api: Build the api",
            SwarmItemState::Claimed,
            Some(&long),
            &[],
        ),
        swarm_item(
            "store: Build the store",
            SwarmItemState::Claimed,
            Some("ada@example.com"),
            &[],
        ),
        swarm_item(
            "docs: Build the docs",
            SwarmItemState::Claimed,
            Some(" \t"),
            &[],
        ),
    ]);
    assert_eq!(
        items.items[0].owner.as_deref(),
        Some("w".repeat(MAX_OWNER_CHARS).as_str())
    );
    assert_eq!(items.items[1].owner, None);
    assert_eq!(items.items[2].owner, None);
}

#[test]
fn the_cost_is_the_observed_tokens_within_the_board_bound() {
    let mut run = view(vec![]);
    assert_eq!(cost_of(&run), 1234);
    run.observed_tokens = u64::MAX;
    assert_eq!(cost_of(&run), MAX_BUDGET_TOKENS);
}
