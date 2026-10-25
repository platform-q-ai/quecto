//! Epic #2705 B8: what a client saw of a run crosses the wire whole, and
//! the server's stated ports answer only for that run and only what was
//! read before the write.
use std::sync::Mutex;

use serde_json::json;

use super::*;
use crate::application::project_board::dto::{
    SwarmEnd, SwarmEvidence, SwarmItemState, SwarmItemView,
};

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

/// A view with every end, state and optional field spelled.
fn view() -> SwarmRunView {
    SwarmRunView {
        end: SwarmEnd::BudgetExhausted,
        observed_tokens: 1234,
        items: vec![
            SwarmItemView {
                title: "schema: Schema".into(),
                state: SwarmItemState::Blocked,
                owner: Some("w-1".into()),
                evidence: vec![SwarmEvidence {
                    artifact: "commit".into(),
                    revision: "abc123abc123".into(),
                }],
                stood_before_death: Some(SwarmItemState::Submitted),
            },
            SwarmItemView {
                title: "api: Api".into(),
                state: SwarmItemState::Completed,
                owner: None,
                evidence: vec![],
                stood_before_death: None,
            },
        ],
    }
}

struct Seen;

impl RunLiveness for Seen {
    fn running_coordinator(&self, task: &Slug, run: u32) -> Option<String> {
        (task.as_str() == "a" && run == 2).then(|| "7f3a".into())
    }
}

struct Kept;

impl SwarmRunReports for Kept {
    fn ended_run(&self, _: &Slug, _: u32) -> Option<SwarmRunView> {
        Some(view())
    }

    fn left_run(&self, _: &Slug, _: u32) -> Option<SwarmRunView> {
        None
    }

    fn recorded(&self, _: &Slug, _: u32) {}
}

fn wire(facts: &RunFacts) -> serde_json::Value {
    serde_json::to_value(facts).unwrap()
}

fn read(wire: serde_json::Value) -> Result<RunFacts, serde_json::Error> {
    serde_json::from_value(wire)
}

#[test]
fn facts_cross_the_wire_whole() {
    let task = slug("a");
    let gathered = RunFacts::gathered((&task, 2), Some(&Seen), Some(&Kept));
    assert_eq!(gathered.liveness, SeenLiveness::Seen(Some("7f3a".into())));
    assert_eq!(read(wire(&gathered)).unwrap(), gathered);
    let sent = wire(&gathered);
    assert_eq!(sent["reports"]["ended"]["end"], json!("budget-exhausted"));
    assert_eq!(
        sent["reports"]["ended"]["items"][0]["stood_before_death"],
        json!("submitted")
    );
    let unseen = RunFacts::unseen();
    assert_eq!(read(wire(&unseen)).unwrap(), unseen);
    assert_eq!(
        wire(&unseen),
        json!({"liveness": "unseen", "reports": null})
    );
}

#[test]
fn facts_not_shaped_as_facts_are_refused() {
    let task = slug("a");
    let mut sent = wire(&RunFacts::gathered((&task, 2), Some(&Seen), Some(&Kept)));
    sent["reports"]["ended"]["end"] = json!("exploded");
    assert!(read(sent).is_err(), "an unknown end");
    for wrong in [
        json!({"liveness": "unseen", "reports": null, "extra": 1}),
        json!({"liveness": {"seen": 7}, "reports": null}),
        json!({"liveness": "unseen", "reports": {"ended": {"end": "failed", "items": [],
                                                           "observed_tokens": -1}}}),
        json!([]),
    ] {
        assert!(read(wrong.clone()).is_err(), "{wrong}");
    }
}

#[test]
fn the_stated_ports_answer_for_their_own_run_only() {
    let task = slug("a");
    let facts = RunFacts::gathered((&task, 2), Some(&Seen), Some(&Kept));
    let liveness = facts.liveness((&task, 2)).expect("the client looked");
    assert_eq!(liveness.running_coordinator(&task, 2), Some("7f3a".into()));
    assert_eq!(liveness.running_coordinator(&task, 1), None);
    let reports = facts.reports((&task, 2)).expect("the client keeps boards");
    assert_eq!(reports.ended_run(&task, 2), Some(view()));
    assert_eq!(reports.ended_run(&slug("b"), 2), None);
    assert_eq!(reports.left_run(&task, 2), None);
    let unseen = RunFacts::unseen();
    assert!(unseen.liveness((&task, 2)).is_none() && unseen.reports((&task, 2)).is_none());
}

/// A remote holding every commit but `missing`, recording what it is asked.
struct Remote {
    missing: Vec<String>,
    asks: Mutex<Vec<Vec<String>>>,
}

impl RemoteRevisions for Remote {
    fn not_on_remote(&self, revisions: &[String]) -> Result<Vec<String>, BoardStoreError> {
        self.asks.lock().unwrap().push(revisions.to_vec());
        let missing = revisions
            .iter()
            .filter(|revision| self.missing.contains(revision));
        Ok(missing.cloned().collect())
    }

    fn commit_id_hex_len(&self) -> Result<usize, BoardStoreError> {
        Ok(40)
    }
}

#[test]
fn stated_revisions_answer_only_what_was_read_before_the_write() {
    let (abc, def) = ("abc123abc123".to_string(), "def456def456".to_string());
    let remote = Remote {
        missing: vec![def.clone()],
        asks: Mutex::default(),
    };
    let short = "abc12".to_string();
    let stated = StatedRemoteRevisions::read(&remote, &[abc.clone(), def.clone(), short.clone()]);
    assert_eq!(
        *remote.asks.lock().unwrap(),
        [vec![abc.clone(), def.clone()]],
        "once, only what names a commit"
    );
    assert_eq!(stated.commit_id_hex_len(), Ok(40));
    assert_eq!(
        stated.not_on_remote(&[abc.clone(), def.clone()]),
        Ok(vec![def])
    );
    assert_eq!(stated.not_on_remote(&[abc]), Ok(vec![]));
    assert!(matches!(
        stated.not_on_remote(&[short]),
        Err(BoardStoreError::Unavailable(_))
    ));
    let unread = StatedRemoteRevisions::unread("not read in time");
    assert_eq!(
        unread.commit_id_hex_len(),
        Err(BoardStoreError::Unavailable("not read in time".into()))
    );
}

fn kept(items: Vec<SwarmItemView>) -> RunFacts {
    let ended = SwarmRunView {
        end: SwarmEnd::Succeeded,
        items,
        observed_tokens: 0,
    };
    RunFacts {
        liveness: SeenLiveness::Unseen,
        reports: Some(KeptViews {
            ended: Some(ended),
            left: None,
        }),
    }
}

/// Review rounds 1 and 2: the server reads the remote only for the commits
/// the evidence of the plan's items names; past `MAX_ITEMS × MAX_PER_ITEM`
/// it refuses up front, saying so.
#[test]
fn only_the_plans_evidence_is_named_and_past_its_bound_refused() {
    use crate::application::project_board::fold_back::tests::{plan, swarm_item};
    let done = SwarmItemState::Completed;
    let facts = kept(vec![
        swarm_item("api: Build the api", done, None, &[("c", "abc123abc123")]),
        swarm_item("extra: Not planned", done, None, &[("c", "0ff0ff0ff0ff")]),
    ]);
    assert_eq!(
        facts.named_commits(&plan()),
        Ok(vec!["abc123abc123".into()])
    );
    let many: Vec<String> = (0..3000).map(|n| format!("{n:012x}")).collect();
    let evidence: Vec<(&str, &str)> = many.iter().map(|id| ("c", id.as_str())).collect();
    let store = "store: Build the store";
    let facts = kept(vec![swarm_item(store, done, None, &evidence)]);
    let refused = facts.named_commits(&plan()).unwrap_err();
    assert!(
        refused.contains("3000 commits, more than the 2048"),
        "{refused}"
    );
}

/// Review rounds 1 and 2: facts past the request's bytes are sent smaller:
/// first without the board the run was left on beside its ended one; an
/// end that is no success then without its evidence, then its boards; a
/// success keeps only evidence naming commits, else is refused here.
#[test]
fn facts_past_their_bytes_are_sent_smaller() {
    use crate::application::project_board::fold_back::tests::swarm_item;
    let long = "x".repeat(200);
    let notes: Vec<(&str, &str)> = (0..100).map(|_| ("notes", long.as_str())).collect();
    let done = SwarmItemState::Completed;
    let mut facts = kept(vec![swarm_item("api: Build the api", done, None, &notes)]);
    let small = facts.clone();
    let views = facts.reports.as_mut().unwrap();
    views.left = views.ended.clone();
    let bytes = serde_json::to_vec(&small).unwrap().len();
    assert_eq!(
        facts.clone().fitting(true, bytes),
        Ok(small.clone()),
        "left goes first"
    );
    let fits = serde_json::to_vec(&facts).unwrap().len();
    assert_eq!(
        facts.clone().fitting(true, fits),
        Ok(facts.clone()),
        "it fits"
    );
    let left = facts.clone().fitting(false, 2_000).unwrap();
    let items = left.reports.unwrap().ended.unwrap().items;
    assert!(
        items.iter().all(|item| item.evidence.is_empty()),
        "no evidence"
    );
    assert_eq!(facts.clone().fitting(false, 100).unwrap().reports, None);
    let commit = [("c", "abc123abc123")];
    let named = kept(vec![swarm_item("api: Build the api", done, None, &commit)]);
    let mut noisy = named.clone();
    let item = &mut noisy
        .reports
        .as_mut()
        .unwrap()
        .ended
        .as_mut()
        .unwrap()
        .items[0];
    item.evidence.extend(
        small.reports.unwrap().ended.unwrap().items[0]
            .evidence
            .clone(),
    );
    let named_bytes = serde_json::to_vec(&named).unwrap().len();
    assert_eq!(
        noisy.clone().fitting(true, named_bytes),
        Ok(named),
        "commits kept"
    );
    assert_eq!(noisy.fitting(true, 50), Err(TOO_LARGE.to_string()));
}
