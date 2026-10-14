//! Epic #2481 T3: one `board_op` record per call, holding the op, its
//! outcome, code, rule, field and fault as the code spells them, and never
//! the request's text.
use std::sync::{Arc, Mutex};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::json;

use super::*;
use crate::application::project_board::operations::NoPullRequestHost;
use crate::application::project_board::operations::query_listings::{QueryListings, assembled};
use crate::application::project_board::ports::{BoardHead, ProjectBoardOpLog, ProjectBoardStore};
use crate::application::project_board::telemetry::RecordingSwitch;
use crate::application::project_board::use_cases::{QueryBoard, RunBoardOperation};
use crate::domain::audit::AuditEvent;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use crate::interface::tools::project_board::run_recorded;

/// The records written, in order.
#[derive(Default)]
pub(crate) struct Records(Mutex<Vec<BoardOpRecord>>);

impl ProjectBoardOpLog for Records {
    fn record(&self, record: BoardOpRecord) {
        self.0.lock().unwrap().push(record);
    }
}

impl Records {
    pub(crate) fn taken(&self) -> Vec<BoardOpRecord> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

/// An outcome as its record's line holds it: the kinds by their names, so
/// a kind missing from `KNOWN_KINDS` shows as `other` here.
fn outcome_line(outcome: &OpOutcome) -> Value {
    serde_json::to_value(outcome).unwrap()
}

/// A recording into `records`, as a switched-on session's would be.
pub(crate) fn recording_into(records: &Arc<Records>) -> BoardRecording {
    let log: Arc<dyn ProjectBoardOpLog> = records.clone();
    let recording = BoardRecording::when(RecordingSwitch::resolved(true), || Some(log));
    recording.expect("on, with a log")
}

/// [`recording_into`], shared as a call takes it.
pub(crate) fn recording(records: &Arc<Records>) -> Arc<BoardRecording> {
    Arc::new(recording_into(records))
}

/// The operations over a bootstrapped board in memory.
fn board() -> RunBoardOperation {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    assembled(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .unwrap()
}

/// Runs each request on `operations`, one call each, and gives each
/// call's records as their `board_op` lines.
fn recorded(operations: &RunBoardOperation, requests: &[String]) -> Vec<Value> {
    let records = Arc::new(Records::default());
    let mut lines = Vec::new();
    for text in requests {
        let mut call = BoardCall::start(Some(recording(&records)), FrontEnd::Cli, Client::Cli);
        let answer = run_recorded(operations, text, &mut call);
        call.finish(&answer, false);
        let mut taken = records.taken();
        assert_eq!(taken.len(), 1, "one record per call: {text}");
        let line = AuditEvent::BoardOp(Box::new(taken.remove(0)));
        lines.push(serde_json::to_value(line).unwrap());
    }
    lines
}

fn one(operations: &RunBoardOperation, request: Value) -> Value {
    recorded(operations, &[request.to_string()]).remove(0)
}

#[test]
fn a_call_records_its_op_access_task_and_outcome() {
    let board = board();
    let created = one(
        &board,
        json!({"op": "create", "task": "a", "title": "A", "status": "ready"}),
    );
    assert_eq!(created["op"], json!("create"));
    assert_eq!(created["access"], json!("write"));
    assert_eq!(created["outcome"], json!("ok"));
    assert_eq!(created["task"], json!("a"));
    assert_eq!(
        (created["front_end"].clone(), created["client"].clone()),
        (json!("cli"), json!("cli"))
    );
    assert!(created["result_bytes"].as_u64().unwrap() > 0, "{created}");
    assert!(created.get("duration_us").is_some(), "{created}");
    let linked = one(&board, json!({"op": "link_pr", "task": "a", "pr": 7}));
    assert_eq!(
        (
            linked["outcome"].clone(),
            linked["code"].clone(),
            linked["rule"].clone(),
            linked["pr"].clone()
        ),
        (json!("refused"), json!("rule"), json!("no_claim"), json!(7)),
        "{linked}"
    );
    let missing = one(&board, json!({"op": "show", "task": "b"}));
    assert_eq!(
        (missing["code"].clone(), missing.get("rule")),
        (json!("not_found"), None)
    );
    assert_eq!(missing["access"], json!("read"));
}

/// A refused request records its op as far as it was read, and the
/// fault its refusal carries (`request_tests.rs` holds every fault).
#[test]
fn a_refused_request_records_its_field_and_fault() {
    let board = board();
    let requests = [
        r#"{"op": "show"}"#.to_string(),
        r#"{"op": "show" "#.into(),
        r#"{"op": "Drop Tables"}"#.into(),
    ];
    let wants = [
        ("show", "read", "task", "missing"),
        ("unknown", "read", "request", "not_json"),
        ("unknown", "read", "op", "invalid_value"),
    ];
    for (line, want) in recorded(&board, &requests).iter().zip(wants) {
        let (op, access, field, fault) = want;
        assert_eq!(line["code"], json!("invalid_request"), "{line}");
        let got = (&line["op"], &line["access"], &line["field"], &line["fault"]);
        assert_eq!(
            got,
            (&json!(op), &json!(access), &json!(field), &json!(fault))
        );
    }
    let create = one(&board, json!({"op": "create", "task": "a"}));
    assert_eq!(
        (&create["op"], &create["access"]),
        (&json!("create"), &json!("write"))
    );
}

/// A key no schema of the op names is `other`, counted, even when it is
/// a field another op takes.
#[test]
fn a_stray_request_key_is_recorded_as_other() {
    let board = board();
    let stray = one(&board, json!({"op": "show", "task": "a", "title": "A"}));
    assert_eq!(
        (&stray["field"], &stray["fault"], &stray["unexpected_count"]),
        (&json!("other"), &json!("unexpected"), &json!(1)),
        "{stray}"
    );
    let strays = one(&board, json!({"op": "list", "Secret Key": 1, "title": 2}));
    assert_eq!(
        (&strays["field"], &strays["unexpected_count"]),
        (&json!("other"), &json!(2))
    );
}

/// Off means off: an unrecorded call notes nothing and times nothing.
#[test]
fn an_unrecorded_call_writes_nothing() {
    let mut call = BoardCall::unrecorded();
    let answer = run_recorded(&board(), r#"{"op": "list"}"#, &mut call);
    assert_eq!(call.built(|| 7), 7);
    assert!(call.open.is_none());
    call.finish(&answer, true);
}

/// #2481 M4a: an answered `show` records how many plan versions and runs
/// it gave, as integers; a listing, and a refused show, record neither.
#[test]
fn a_show_records_its_plan_and_run_counts_only() {
    use crate::application::project_board::ports::BoardArtifact;
    use crate::infrastructure::test_support::board_artifacts::{plan, run};
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let operations = assembled(
        Arc::new(store.client()),
        Arc::new(SystemBoardClock::default()),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .unwrap();
    let created = json!({"op": "create", "task": "a", "title": "A", "status": "ready"});
    run_recorded(
        &operations,
        &created.to_string(),
        &mut BoardCall::unrecorded(),
    );
    let head = store.read().unwrap().head;
    let seeded = [
        BoardArtifact::Plan(plan("a", 1)),
        BoardArtifact::Run(run("a", 1)),
        BoardArtifact::Run(run("a", 2)),
    ];
    store.write(&head, &seeded, "board: seed").unwrap();
    let lines = recorded(
        &operations,
        &[
            json!({"op": "show", "task": "a"}).to_string(),
            json!({"op": "show", "task": "b"}).to_string(),
            json!({"op": "list"}).to_string(),
        ],
    );
    assert_eq!(
        (&lines[0]["plans"], &lines[0]["runs"]),
        (&json!(1), &json!(2))
    );
    for line in &lines[1..] {
        assert_eq!(
            (line.get("plans"), line.get("runs")),
            (None, None),
            "{line}"
        );
    }
}

/// `head` is a commit id git produced, or nothing.
#[test]
fn a_head_is_recorded_only_as_a_full_commit_id() {
    let records = Arc::new(Records::default());
    for head in [
        "0123456789abcdef0123456789abcdef01234567",
        "r1",
        "Secret Head",
    ] {
        let mut call = BoardCall::start(Some(recording(&records)), FrontEnd::Tool, Client::Agent);
        call.named(Some("clear_set_aside"));
        let outcome = BoardOutcome::Cleared {
            head: BoardHead(head.into()),
        };
        let answer = call.answer("clear_set_aside", &outcome, None);
        call.finish(&answer, false);
    }
    let heads: Vec<Option<String>> = records
        .taken()
        .into_iter()
        .map(|record| record.head.map(|head| head.as_str().to_owned()))
        .collect();
    assert_eq!(
        heads,
        [
            Some("0123456789abcdef0123456789abcdef01234567".into()),
            None,
            None
        ]
    );
}

#[test]
fn a_listing_records_what_its_answer_showed_and_left_out() {
    let records = Arc::new(Records::default());
    let mut call = BoardCall::start(Some(recording(&records)), FrontEnd::Tool, Client::Agent);
    call.named(Some("list"));
    let answer = json!({"ok": true, "op": "list", "tasks": [{}, {}], "truncated": {"shown": 2, "more": 5},
                        "quarantined": [{}], "quarantined_more": 3});
    call.finish(&answer, true);
    let record = records.taken().remove(0);
    let counts = ListingCounts {
        listed: 7,
        shown: 2,
        truncated_more: 5,
        set_aside: 4,
        set_aside_more: 3,
    };
    assert_eq!((record.listing, record.cancelled), (Some(counts), true));
    // A refused listing, or another op's answer, has no counts.
    let mut show = BoardCall::start(Some(recording(&records)), FrontEnd::Tool, Client::Agent);
    show.named(Some("show"));
    show.finish(&answer, false);
    assert_eq!(records.taken().remove(0).listing, None);
}

/// Text from outside, `rng`'s: a capital first, so it is never shaped as a
/// kind, and long enough that no count or id holds it by chance.
pub(crate) fn outside_text(rng: &mut StdRng) -> String {
    const REST: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 _-./:@";
    let first = char::from(rng.gen_range(b'A'..=b'Z'));
    let length = rng.gen_range(11..40);
    let rest = (0..length).map(|_| char::from(REST[rng.gen_range(0..REST.len())]));
    std::iter::once(first).chain(rest).collect()
}

/// Property (T3): random titles, descriptions and keys, accepted or
/// refused, never appear in a serialized record.
#[test]
fn no_request_text_ever_reaches_a_record() {
    let board = board();
    let seed: u64 = rand::thread_rng().r#gen();
    let mut rng = StdRng::seed_from_u64(seed);
    for case in 0..64 {
        let (title, description, key) = (
            outside_text(&mut rng),
            outside_text(&mut rng),
            outside_text(&mut rng),
        );
        let task = format!("t-{case}");
        let requests = [
            json!({"op": "create", "task": task, "title": title, "description": description}),
            json!({"op": "update_draft", "task": task, "title": description, "description": title}),
            json!({"op": "create", "task": format!("u-{case}"), "title": title, key.clone(): description}),
            json!({"op": "show", "task": title}),
            json!({"op": key.clone()}),
            json!({key.clone(): title}),
            json!({"op": "clear_set_aside", "path": title, "object": "0".repeat(40),
                   "repeated": {key.clone(): {"drop": [description]}}}),
            json!({"op": "move", "task": task, "to": description}),
            // A plan's nested text and keys (#2481 M4b).
            json!({"op": "propose_plan", "task": task, "approach": description,
                   "items": [{"id": "i", "title": title, "criteria": [description]}],
                   "team": {"roles": [{"name": "w", "model": "m", "effort": "low", "members": 1}],
                            "image": title}}),
            json!({"op": "propose_plan", "task": task, "approach": "",
                   "items": [{"id": "i", "title": "T", key.clone(): title}]}),
            json!({"op": "edit_plan", "task": task, "version": 1, "approach": title,
                   "items": [{"id": "i", "title": description}]}),
        ];
        let texts: Vec<String> = requests.iter().map(Value::to_string).collect();
        for (line, text) in recorded(&board, &texts).iter().zip(&texts) {
            let line = line.to_string();
            for outside in [&title, &description, &key] {
                assert!(
                    !line.contains(outside.as_str()),
                    "seed {seed}: {outside:?} from {text} reached {line}"
                );
            }
        }
    }
}

/// #2537's `id_in_use` refusal records its code, and names no field.
#[test]
fn an_id_in_use_refusal_records_its_code() {
    use crate::application::project_board::dto::{IdUser, ProjectBoardError};
    use crate::domain::project_board::value_objects::slug::Slug;
    let records = Arc::new(Records::default());
    let mut call = BoardCall::start(Some(recording(&records)), FrontEnd::Tool, Client::Agent);
    call.named(Some("create"));
    let error = ProjectBoardError::IdInUse {
        id: Slug::parse("a").unwrap(),
        by: IdUser::Run(1),
    };
    let answer = call.refuse_as_board("create", &error);
    assert_eq!(answer["error"]["code"], json!("id_in_use"));
    call.finish(&answer, false);
    let record = records.taken().remove(0);
    assert_eq!(
        (outcome_line(&record.outcome), record.fault),
        (json!({"outcome": "refused", "code": "id_in_use"}), None)
    );
}

/// Review round 1: the board's refusals that name a schema field record
/// it: a task the board refuses (`invalid_task`) and a rule's `invalid`.
#[test]
fn a_refusal_naming_a_schema_field_records_it() {
    let board = board();
    let invalid_task = one(&board, json!({"op": "create", "task": "a", "title": ""}));
    assert_eq!(
        (
            &invalid_task["code"],
            &invalid_task["field"],
            &invalid_task["fault"]
        ),
        (
            &json!("invalid_task"),
            &json!("title"),
            &json!("invalid_value")
        ),
        "{invalid_task}"
    );
    one(&board, json!({"op": "create", "task": "b", "title": "B"}));
    let invalid_rule = one(
        &board,
        json!({"op": "update_draft", "task": "b", "title": "Two\nlines"}),
    );
    assert_eq!(
        (
            &invalid_rule["code"],
            &invalid_rule["rule"],
            &invalid_rule["field"],
            &invalid_rule["fault"]
        ),
        (
            &json!("rule"),
            &json!("invalid"),
            &json!("title"),
            &json!("invalid_value")
        ),
        "{invalid_rule}"
    );
    // A refusal that names no schema field records none.
    let missing = one(&board, json!({"op": "show", "task": "c"}));
    assert_eq!((missing.get("field"), missing.get("fault")), (None, None));
}

/// Review round 1: the answer is the truth. An answer and its note never
/// disagree (each is made by the same call); were they to, the record
/// takes the answer's outcome (a release build does not panic).
#[test]
fn the_answer_decides_an_outcome_its_note_disagrees_with() {
    let refused = json!({"ok": false, "op": "show", "error": {"code": "not_found"}});
    let ok = json!({"ok": true, "op": "show"});
    let noted_refusal = OpOutcome::Refused {
        code: StaticKind::new("stale"),
        rule: None,
    };
    let line = |noted: OpOutcome, answer: &Value| outcome_line(&answered_outcome(noted, answer));
    assert_eq!(line(OpOutcome::Ok, &ok), json!({"outcome": "ok"}));
    assert_eq!(
        line(noted_refusal.clone(), &refused),
        json!({"outcome": "refused", "code": "stale"}),
        "an agreeing answer keeps the note"
    );
    assert_eq!(line(noted_refusal, &ok), json!({"outcome": "ok"}));
    let unknown = json!({"ok": false, "error": {"code": "Secret Code"}});
    assert_eq!(
        line(OpOutcome::Ok, &refused),
        json!({"outcome": "refused", "code": "not_found"})
    );
    assert_eq!(
        line(OpOutcome::Ok, &unknown),
        json!({"outcome": "refused", "code": "other"})
    );
}

/// Review rounds 1 and 2: a disagreement is a bug, caught in a debug
/// build; the record (with the answer's outcome) is written first, and the
/// message holds only the answer's `ok` and the noted outcome, never the
/// answer's text.
#[test]
fn a_disagreeing_answer_is_recorded_then_caught_in_debug_builds() {
    let records = Arc::new(Records::default());
    let mut call = BoardCall::start(Some(recording(&records)), FrontEnd::Cli, Client::Cli);
    call.named(Some("show"));
    let answer = json!({"ok": false, "error": {"code": "not_found", "message": "Secret Title"}});
    let finished =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| call.finish(&answer, false)));
    assert_eq!(
        finished.is_err(),
        cfg!(debug_assertions),
        "only a debug build asserts"
    );
    if let Err(payload) = finished {
        let message = crate::application::tool_panic_scope::payload_message(payload.as_ref());
        assert!(message.contains("disagrees with its note"), "{message}");
        assert!(!message.contains("Secret Title"), "{message}");
    }
    let written = records.taken();
    assert_eq!(
        written.len(),
        1,
        "the record is written before the assertion"
    );
    assert_eq!(
        outcome_line(&written[0].outcome),
        json!({"outcome": "refused", "code": "not_found"})
    );
}

/// Review round 2: a call dropped unfinished without a panic is recorded
/// `cancelled`, as far as it was noted, never `internal`.
#[test]
fn a_call_dropped_unfinished_is_recorded_cancelled() {
    let records = Arc::new(Records::default());
    let mut call = BoardCall::start(Some(recording(&records)), FrontEnd::Tool, Client::Agent);
    call.named(Some("show"));
    drop(call);
    let written = records.taken();
    assert_eq!(written.len(), 1);
    assert_eq!(
        (written[0].op, &written[0].outcome, written[0].cancelled),
        (OpKind::Show, &OpOutcome::Ok, true)
    );
}

/// Epic #2481 T4: a recorded call holds what the use cases saw, and a
/// failed completion's error code as the answer spells it.
#[test]
fn a_call_records_what_its_use_case_saw() {
    let board = board();
    let lines = recorded(
        &board,
        &[
            json!({"op": "create", "task": "a", "title": " A ", "status": "ready"}).to_string(),
            json!({"op": "move", "task": "a", "to": "claimed"}).to_string(),
            json!({"op": "link_pr", "task": "a", "pr": 7}).to_string(),
        ],
    );
    // A create that gave no description tells none.
    assert_eq!(lines[0]["normalised"], json!({"title": ["edge_space"]}));
    assert_eq!(
        (
            lines[0]["status_to"].clone(),
            lines[0]["attempts"].clone(),
            lines[0]["lost"].clone()
        ),
        (json!("ready"), json!(1), json!(0))
    );
    assert_eq!(
        (
            lines[1]["status_from"].clone(),
            lines[1]["status_to"].clone()
        ),
        (json!("ready"), json!("claimed"))
    );
    let linked = &lines[2];
    assert_eq!(linked["claim"]["holder_is_writer"], json!(true), "{linked}");
    assert_eq!(
        (
            linked["completion"].clone(),
            linked["completion_code"].clone(),
            linked["prs_linked"].clone()
        ),
        (json!("failed"), json!("no_pull_request_host"), json!(1)),
        "{linked}"
    );
}

/// #2481 M4b: a plan's op records the version, its task items and team
/// roles, and what the normaliser changed, by kind; a nested fault, or a
/// plan's own schema refusal, records the top field it lies under.
#[test]
fn a_plan_op_records_its_version_counts_and_normalised_kinds() {
    let board = board();
    let team = json!({"roles": [{"name": "w", "model": "m", "effort": "low", "members": 2}]});
    let lines = recorded(
        &board,
        &[
            json!({"op": "create", "task": "a", "title": "A"}).to_string(),
            json!({"op": "propose_plan", "task": "a", "approach": "Do.\r\n",
                   "items": [{"id": "i", "title": " I", "criteria": ["x\ty"]}], "team": team})
            .to_string(),
            json!({"op": "edit_plan", "task": "a", "version": 1, "team": null}).to_string(),
            json!({"op": "edit_plan", "task": "a", "version": 1, "items": [{"id": "i", "title": "Two\nlines"}]})
                .to_string(),
            json!({"op": "propose_plan", "task": "a", "approach": "", "items": [{"id": "i", "title": 7}]})
                .to_string(),
        ],
    );
    let facts = |line: &Value| {
        let field = |key: &str| line.get(key).cloned().unwrap_or(Value::Null);
        [
            "op",
            "access",
            "plan_version",
            "items",
            "roles",
            "normalised",
            "attempts",
        ]
        .map(field)
    };
    assert_eq!(
        facts(&lines[1]),
        [
            json!("propose_plan"),
            json!("write"),
            json!(1),
            json!(1),
            json!(1),
            json!({"approach": ["crlf", "edge_blank_lines"], "items": ["tab", "edge_space"]}),
            json!(1),
        ]
    );
    // An edit that gives no text tells nothing normalised.
    let edited = facts(&lines[2]);
    assert_eq!(edited[2..6], [json!(1), json!(1), json!(0), Value::Null]);
    let invalid = &lines[3];
    assert_eq!(
        [
            &invalid["code"],
            &invalid["rule"],
            &invalid["field"],
            &invalid["plan_version"]
        ],
        [
            &json!("rule"),
            &json!("invalid_plan"),
            &json!("items"),
            &json!(1)
        ],
        "{invalid}"
    );
    let wrong = &lines[4];
    assert_eq!(
        [&wrong["code"], &wrong["field"], &wrong["fault"]],
        [
            &json!("invalid_request"),
            &json!("items"),
            &json!("wrong_type")
        ],
        "{wrong}"
    );
}

/// A board clock that counts the reads of its monotonic time.
#[derive(Default)]
struct CountingClock {
    clock: SystemBoardClock,
    elapsed_reads: std::sync::atomic::AtomicUsize,
}

impl crate::application::project_board::ports::BoardClock for CountingClock {
    fn now(
        &self,
    ) -> Result<
        crate::domain::project_board::value_objects::timestamp::Timestamp,
        crate::application::project_board::ports::BoardClockError,
    > {
        self.clock.now()
    }

    fn reference(
        &self,
    ) -> Option<crate::domain::project_board::value_objects::timestamp::Timestamp> {
        None
    }

    fn reference_status(&self) -> crate::application::project_board::ports::ReferenceStatus {
        crate::application::project_board::ports::ReferenceStatus::NoReference("counted".into())
    }

    fn pause(&self, _attempt: usize) {}

    fn elapsed(&self) -> Duration {
        self.elapsed_reads
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Duration::ZERO
    }
}

/// Epic #2481 T4, owner condition: an unrecorded call notes nothing and
/// times nothing, even through a write that lost a race; the same write
/// recorded times its pause.
#[test]
fn an_unrecorded_write_reads_no_monotonic_time() {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let clock = Arc::new(CountingClock::default());
    let operations = assembled(
        Arc::new(store.client()),
        clock.clone(),
        Arc::new(QueryListings(QueryBoard::new(Arc::new(store.client())))),
        Err(NoPullRequestHost::NotAPullRequestHost),
    )
    .unwrap();
    let create = |task: &str, call: &mut BoardCall| {
        let request = json!({"op": "create", "task": task, "title": "T", "status": "ready"});
        let answer = run_recorded(&operations, &request.to_string(), call);
        assert_eq!(answer["ok"], json!(true), "{answer}");
    };
    let lose_once = || {
        store.before_next_write(|board| {
            let read = board.read().unwrap();
            board
                .write_tasks(&read.head, &read.tasks, "another writer")
                .unwrap();
        });
    };
    let reads = || {
        clock
            .elapsed_reads
            .load(std::sync::atomic::Ordering::SeqCst)
    };
    create("z", &mut BoardCall::unrecorded());
    lose_once();
    create("a", &mut BoardCall::unrecorded());
    assert_eq!(store.writes(), 3, "two creates and the write each lost to");
    assert_eq!(reads(), 0, "nothing timed while unrecorded");
    let records = Arc::new(Records::default());
    let mut call = BoardCall::start(Some(recording(&records)), FrontEnd::Cli, Client::Cli);
    lose_once();
    create("b", &mut call);
    assert_eq!(reads(), 2, "the recorded write timed its one pause");
}

/// Review round 2: a completion a rule failed names the rule as well as
/// its code, as the answer spells them.
#[test]
fn a_rule_failed_completion_records_its_rule() {
    let board = board();
    let created = json!({"op": "create", "task": "a", "title": "A", "status": "ready"});
    run_recorded(&board, &created.to_string(), &mut BoardCall::unrecorded());
    let BoardOutcome::Shown(shown, _) = board
        .execute(&BoardRequest::Show(
            crate::domain::project_board::value_objects::slug::Slug::parse("a").unwrap(),
            crate::application::project_board::operations::PlanView::default(),
        ))
        .unwrap()
    else {
        panic!("shown")
    };
    let task = shown.task;
    let records = Arc::new(Records::default());
    let mut call = BoardCall::start(Some(recording(&records)), FrontEnd::Tool, Client::Agent);
    call.named(Some("link_pr"));
    let facts = crate::application::project_board::telemetry::OpFacts {
        completion: Some(
            crate::application::project_board::telemetry::completion_facts(
                CompletionKind::Failed,
                1,
                None,
            ),
        ),
        ..Default::default()
    };
    facts.fill(&mut call.open.as_mut().unwrap().record);
    let failed = crate::application::project_board::operations::Completion::Failed(Box::new(
        ProjectBoardError::Rule(BoardRuleError::NoPrs),
    ));
    let outcome = BoardOutcome::Changed {
        task,
        completion: Some(failed),
    };
    let answer = call.answer("link_pr", &outcome, None);
    call.finish(&answer, false);
    let line = serde_json::to_value(records.taken().remove(0)).unwrap();
    assert_eq!(
        (
            line["completion"].clone(),
            line["completion_code"].clone(),
            line["completion_rule"].clone()
        ),
        (json!("failed"), json!("rule"), json!("no_prs")),
        "{line}"
    );
}
