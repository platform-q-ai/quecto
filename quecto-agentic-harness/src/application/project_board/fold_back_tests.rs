use super::*;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::plan::{Plan, PlanFields, PlanItem};
use crate::domain::project_board::entities::run::{Run, RunFields};
use crate::domain::project_board::entities::task_parts::MAX_BUDGET_TOKENS;
use crate::domain::project_board::services::runs::fold_back;
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
    run_items(&plan(), &view(items), &MarkdownTextCells, Evidence::Kept)
}

/// The one evidence line `artifact at revision` makes, if it is kept.
fn kept_line(artifact: &str, revision: &str) -> Option<String> {
    let items = folded(vec![swarm_item(
        "api: Build the api",
        SwarmItemState::Completed,
        None,
        &[(artifact, revision)],
    )]);
    items.items[0].evidence.first().cloned()
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
        // Close to a key, but neither it nor its `<slug>: ` prefix.
        swarm_item("Docs: Build the docs", SwarmItemState::Completed, None, &[]),
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
fn owners_come_first_and_the_folded_run_stays_within_its_text_budget() {
    let line = "y".repeat(MAX_LINE_CHARS - " at abc".len());
    // 100 characters of 4 bytes each: owners take 400 bytes an item.
    let owner = "\u{10400}".repeat(MAX_OWNER_CHARS);
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
    let items: Vec<SwarmItemView> = (0..40)
        .map(|n| {
            let title = format!("i{n}: Item {n}");
            let mut item = swarm_item(&title, SwarmItemState::Completed, Some(&owner), &[]);
            // Distinct lines per item: the budget, not distinctness, is in play.
            item.evidence = (0..MAX_PER_ITEM)
                .map(|at| SwarmEvidence {
                    artifact: format!("{at:02}{}", &line[2..]),
                    revision: "abc".into(),
                })
                .collect();
            item
        })
        .collect();
    let folded = run_items(&plan, &view(items), &MarkdownTextCells, Evidence::Kept);
    let owners: usize = folded
        .items
        .iter()
        .filter_map(|item| item.owner.as_ref())
        .map(String::len)
        .sum();
    assert_eq!(owners, 40 * 4 * MAX_OWNER_CHARS);
    let lines: usize = folded
        .items
        .iter()
        .flat_map(|item| &item.evidence)
        .map(String::len)
        .sum();
    assert!(owners + lines <= MAX_RUN_TEXT_BYTES, "{owners} + {lines}");
    assert!(
        owners + lines + MAX_LINE_CHARS > MAX_RUN_TEXT_BYTES,
        "{owners} + {lines}"
    );
    assert_eq!(folded.evidence + folded.dropped, 40 * MAX_PER_ITEM);
    assert!(folded.dropped > 0);
    // The domain's own fold-back takes it: the run's text is within bound.
    let at = Timestamp::parse("2026-10-10T10:00:00Z").unwrap();
    let open = Run::new(RunFields {
        task: slug("board-store"),
        number: 1,
        plan_version: 1,
        started: at.clone(),
        ended: None,
        outcome: None,
        cost: None,
        items: vec![],
    })
    .unwrap();
    let run = fold_back(
        &open,
        &plan,
        RunOutcome::Succeeded,
        Some(1),
        folded.items,
        &at,
    )
    .unwrap();
    assert!(run.fields().text_bytes() <= MAX_RUN_TEXT_BYTES);
}

#[test]
fn evidence_left_out_keeps_states_and_owners_and_counts_every_line() {
    let items = run_items(
        &plan(),
        &view(vec![swarm_item(
            "api: Build the api",
            SwarmItemState::Completed,
            Some("worker-1"),
            &[("src/api.rs", "4f2a9c1"), ("src/lib.rs", "4f2a9c1")],
        )]),
        &MarkdownTextCells,
        Evidence::Left,
    );
    assert_eq!(items.items[0].state, ItemState::Done);
    assert_eq!(items.items[0].owner.as_deref(), Some("worker-1"));
    assert!(items.items.iter().all(|item| item.evidence.is_empty()));
    assert_eq!((items.evidence, items.dropped), (0, 2));
}

#[test]
fn of_several_items_under_one_key_the_most_progressed_then_newest_is_taken() {
    let items = folded(vec![
        // Blocked, then created again and completed.
        swarm_item(
            "api: Build the api",
            SwarmItemState::Completed,
            Some("worker-3"),
            &[],
        ),
        swarm_item(
            "api: Build the api",
            SwarmItemState::Blocked,
            Some("worker-1"),
            &[],
        ),
        // Two claimed: the newer is taken.
        swarm_item(
            "store: Build the store",
            SwarmItemState::Claimed,
            Some("worker-1"),
            &[],
        ),
        swarm_item(
            "store: Build the store",
            SwarmItemState::Claimed,
            Some("worker-2"),
            &[],
        ),
        swarm_item("docs: Build the docs", SwarmItemState::Ready, None, &[]),
        swarm_item(
            "docs: Build the docs",
            SwarmItemState::Submitted,
            Some("worker-4"),
            &[],
        ),
    ]);
    let states: Vec<(ItemState, Option<&str>)> = items
        .items
        .iter()
        .map(|item| (item.state, item.owner.as_deref()))
        .collect();
    assert_eq!(
        states,
        [
            (ItemState::Done, Some("worker-3")),
            (ItemState::Doing, Some("worker-2")),
            (ItemState::Review, Some("worker-4")),
        ]
    );
    assert_eq!((items.unmatched, items.extra), (0, 3));
    let ranked = [
        SwarmItemState::Blocked,
        SwarmItemState::Ready,
        SwarmItemState::Claimed,
        SwarmItemState::Submitted,
        SwarmItemState::Completed,
    ];
    assert!(
        ranked
            .windows(2)
            .all(|pair| pair[0].progress() < pair[1].progress())
    );
}

#[test]
fn a_title_rewritten_after_its_key_prefix_still_matches_its_item() {
    let items = folded(vec![
        swarm_item(
            "api: Build the API, rewritten",
            SwarmItemState::Completed,
            None,
            &[],
        ),
        swarm_item(
            "store: Build the store ",
            SwarmItemState::Claimed,
            None,
            &[],
        ),
        // An exact title wins over a prefix.
        swarm_item(
            "docs: Docs, rewritten",
            SwarmItemState::Completed,
            None,
            &[],
        ),
        swarm_item("docs: Build the docs", SwarmItemState::Ready, None, &[]),
        // Another slug's prefix is not this one's.
        swarm_item(
            "api-v2: Build the api",
            SwarmItemState::Completed,
            None,
            &[],
        ),
        swarm_item("api:Build the api", SwarmItemState::Completed, None, &[]),
    ]);
    let states: Vec<ItemState> = items.items.iter().map(|item| item.state).collect();
    assert_eq!(states, [ItemState::Done, ItemState::Doing, ItemState::Todo]);
    assert_eq!((items.unmatched, items.extra), (0, 3));
}

#[test]
fn secrets_never_reach_the_run() {
    const TOKEN: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let signed = "https://bucket.s3.amazonaws.com/build.tgz?X-Amz-Algorithm=AWS4-HMAC-SHA256\
        &X-Amz-Credential=AKIAIOSFODNN7EXAMPLE%2F20261013%2Fus-east-1%2Fs3%2Faws4_request\
        &X-Amz-Date=20261013T000000Z&X-Amz-Expires=3600&X-Amz-SignedHeaders=host\
        &X-Amz-Signature=deadbeefcafe0123456789";
    let token_url = format!("https://github.com/o/r/pull/7?token={TOKEN}");
    let userinfo = format!("https://ada:{TOKEN}@github.com/o/r");
    let items = folded(vec![
        swarm_item(
            "api: Build the api",
            SwarmItemState::Completed,
            Some(&format!("worker {TOKEN}")),
            &[
                (token_url.as_str(), "4f2a9c1"),
                (TOKEN, "4f2a9c1"),
                ("deployed", "Bearer abcdef0123456789"),
                ("password=hunter2", "main"),
                (signed, "v1"),
                (userinfo.as_str(), "main"),
            ],
        ),
        swarm_item(
            "store: Build the store",
            SwarmItemState::Claimed,
            Some("password=hunter2"),
            &[],
        ),
    ]);
    let written: Vec<&str> = items
        .items
        .iter()
        .flat_map(|item| item.evidence.iter().chain(&item.owner))
        .map(String::as_str)
        .collect();
    for secret in [
        TOKEN,
        "hunter2",
        "abcdef0123456789",
        "AKIA",
        "deadbeef",
        "Amz",
    ] {
        assert!(
            written.iter().all(|line| !line.contains(secret)),
            "{secret}: {written:?}"
        );
    }
    assert_eq!(
        items.items[0].evidence,
        [
            "[link](https://github.com/o/r/pull/7) at 4f2a9c1",
            "[REDACTED] at 4f2a9c1",
            "deployed at [REDACTED]",
            "[REDACTED] at main",
            "[link](https://bucket.s3.amazonaws.com/build.tgz) at v1",
            "[link](https://github.com/o/r) at main",
        ]
    );
    assert_eq!(items.items[0].owner.as_deref(), Some("worker [REDACTED]"));
    assert_eq!(items.items[1].owner.as_deref(), Some("[REDACTED]"));
}

#[test]
fn only_a_whole_address_is_linked_with_its_punctuation_left_outside() {
    let rows = [
        ("https://x.com/a)b", "[link](https://x.com/a%29b)"),
        ("https://x.com/a(b)c", "[link](https://x.com/a%28b%29c)"),
        (
            "see https://x.com/a, then",
            "see [link](https://x.com/a), then",
        ),
        ("<https://x.com/a>", "[link](https://x.com/a)"),
        ("(https://x.com/a)", "([link](https://x.com/a))"),
        ("https://x.com/a.", "[link](https://x.com/a)."),
        ("https://x.com/a#L10", "[link](https://x.com/a#L10)"),
    ];
    for (artifact, line) in rows {
        assert_eq!(
            kept_line(artifact, "abc"),
            Some(format!("{line} at abc")),
            "{artifact}"
        );
    }
    // Not an address in the codec's set: left as written, so the codec
    // refuses the line and it is dropped.
    for artifact in [
        "https://x.com/a`b`",
        "https://x.com/[a]",
        "https://",
        "xhttps://x.com",
    ] {
        let line = kept_line(artifact, "abc");
        assert!(
            line.as_deref().is_none_or(|line| !line.contains("[link]")),
            "{artifact}: {line:?}"
        );
    }
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
    // Cut at the limit, then trimmed again: no space is left at its end.
    let spaced = format!("{} b", "a".repeat(MAX_OWNER_CHARS - 1));
    let items = folded(vec![swarm_item(
        "api: Build the api",
        SwarmItemState::Claimed,
        Some(&spaced),
        &[],
    )]);
    assert_eq!(
        items.items[0].owner.as_deref(),
        Some("a".repeat(MAX_OWNER_CHARS - 1).as_str())
    );
}

#[test]
fn the_cost_is_the_observed_tokens_within_the_board_bound() {
    let mut run = view(vec![]);
    assert_eq!(cost_of(&run), 1234);
    run.observed_tokens = u64::MAX;
    assert_eq!(cost_of(&run), MAX_BUDGET_TOKENS);
}
