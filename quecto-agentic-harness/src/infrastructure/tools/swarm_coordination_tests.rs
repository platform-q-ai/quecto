use super::*;
#[test]
fn malformed_wire_membership_is_rejected_instead_of_silently_skipping_cleanup() {
    for members in [
        json!(null),
        json!([{ "id":"p", "status":"live", "pid":123 }]),
        json!([{ "id":"p", "status":"bogus" }]),
    ] {
        assert!(
            decode(json!({"control_generation":0,"status":"failed","coordinator":"p","deadline":1,"members":members}))
                .is_err()
        );
    }
    assert!(
        decode(json!({"control_generation":0,"status":"unknown","coordinator":"p","deadline":1,"members":[]})).is_err()
    );
}

#[test]
fn paused_snapshot_is_readable_and_does_not_request_terminal_cleanup() {
    let snapshot = decode(
        json!({"control_generation":0,"status":"paused","coordinator":"p","deadline":1,
        "members":[{"id":"p","status":"live"}]}),
    )
    .expect("a paused run must remain inspectable");
    assert!(!snapshot.status.terminal());
}

#[test]
fn control_receipts_require_typed_status_generation_and_budget() {
    for value in [
        json!({}),
        json!({"status":"running"}),
        json!({"status":"unknown","generation":1}),
        json!({"status":"running","generation":-1}),
        json!({"status":"running","generation":1,"budget":{}}),
    ] {
        assert!(SwarmContext::decode_control_receipt(value, false).is_err());
    }
    let receipt =
        SwarmContext::decode_control_receipt(json!({"status":"paused","generation":7}), false)
            .unwrap();
    assert_eq!(receipt.status, RunStatus::Paused);
    assert_eq!(receipt.generation, 7);
}

#[test]
fn each_member_carries_its_launcher_so_settlement_knows_who_ends_it() {
    // #2121: the store records who launched each member; a legacy row or the
    // bootstrapped coordinator has none.
    let snapshot = decode(
        json!({"control_generation":0,"status":"succeeded","coordinator":"p",
        "deadline":1,"members":[
            {"id":"p","status":"live","launcher":null},
            {"id":"w","status":"live","launcher":"p"},
            {"id":"legacy","status":"live"}
        ]}),
    )
    .expect("a closed run is readable");
    let launchers: Vec<_> = snapshot
        .members
        .iter()
        .map(|m| m.launcher.as_deref())
        .collect();
    assert_eq!(launchers, [None, Some("p"), None]);
}

#[test]
fn a_relative_deadline_becomes_now_plus_the_seconds_and_wins_over_an_absolute_one() {
    // #2125: a model need not know the current Unix time to create a run.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    for input in [
        json!({"deadline_in_seconds": 3600}),
        json!({"deadline": 0, "deadline_in_seconds": 3600}),
        json!({"deadline": null, "deadline_in_seconds": 3600}),
    ] {
        let at = absolute_deadline(&input)
            .unwrap()
            .as_f64()
            .expect("a number");
        assert!((now + 3590.0..now + 3610.0).contains(&at), "{input}: {at}");
    }
    assert_eq!(
        absolute_deadline(&json!({"deadline": 42})).unwrap(),
        json!(42)
    );
    assert!(
        absolute_deadline(&json!({})).unwrap().is_null(),
        "the store refuses it"
    );
}

#[test]
fn a_relative_deadline_outside_one_second_to_seven_days_is_refused_by_name() {
    for bad in [json!(0), json!(-5), json!(604_801), json!("3600")] {
        let error = absolute_deadline(&json!({"deadline_in_seconds": bad})).unwrap_err();
        assert!(
            error.to_string().contains("deadline_in_seconds"),
            "{bad}: {error}"
        );
    }
}

/// #2205: `constraints` is optional in meaning and in the schema: omitted
/// or `null` is an empty list; any other value goes to the store as given.
#[test]
fn an_omitted_or_null_constraints_list_is_empty_and_a_given_one_is_passed_as_given() {
    assert_eq!(run_constraints(&json!({"goal": "g"})), json!([]));
    assert_eq!(
        run_constraints(&json!({"goal": "g", "constraints": null})),
        json!([])
    );
    for given in [json!(["no network"]), json!([]), json!("x"), json!(0)] {
        assert_eq!(run_constraints(&json!({"constraints": given})), given);
    }
}

fn store_member(checkout: &std::path::Path) -> SwarmContext {
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    let context = SwarmContext {
        board: crate::composition::swarm::swarm_board(),
        checkout: checkout.to_path_buf(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
    };
    context.join(&this_process(), None, None).unwrap();
    context
}

fn this_process() -> ProcessIdentity {
    let pid = std::process::id();
    ProcessIdentity {
        pid,
        started: crate::infrastructure::tools::swarm_bridge::process_start(pid).unwrap(),
    }
}

fn create_input(constraints: Option<Value>) -> Value {
    let mut input = json!({"goal":"g",
        "criteria":[{"id":"t","kind":"command","description":"pass"}],
        "member_limit":1,"deadline_in_seconds":300});
    if let Some(constraints) = constraints {
        input["constraints"] = constraints;
    }
    input
}

#[test]
fn a_run_is_created_without_constraints_but_not_with_a_wrong_typed_value() {
    for none in [None, Some(json!(null))] {
        let dir = tempfile::tempdir().unwrap();
        let context = store_member(dir.path());
        context
            .create_run(&create_input(none.clone()), &this_process(), None)
            .expect("constraints may be omitted or null");
        let summary = context.summary().unwrap();
        assert_eq!(summary["constraints"], json!([]), "{none:?}: {summary}");
    }

    for wrong in [json!("no network"), json!({}), json!([1])] {
        let dir = tempfile::tempdir().unwrap();
        let context = store_member(dir.path());
        let error = context
            .create_run(&create_input(Some(wrong.clone())), &this_process(), None)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("constraints must be a list of strings"),
            "{wrong}: {error}"
        );
    }
}

/// The Rust board's `_notifications` answers (#2276), both the member list
/// and the `{members, generation}` batch, decode as the wire members this
/// adapter reads from the store.
#[test]
fn rust_board_notifications_decode_as_wire_members() {
    use crate::application::swarm::dto::BoardLocation;
    use crate::composition::swarm::build_swarm_board_handles;
    use crate::infrastructure::tools::swarm_board_dispatch::call;
    let dir = tempfile::tempdir().unwrap();
    let handles = build_swarm_board_handles(
        BoardLocation {
            database: dir.path().join("swarm.sqlite"),
            checkout: dir.path().to_path_buf(),
        },
        None,
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let criteria = json!([{"id": "t", "kind": "command", "description": "d"}]);
    for (member, method, args) in [
        (
            "parent",
            "create_run",
            json!(["g", [], criteria, 3, now + 3_600.0]),
        ),
        ("parent", "_admit", json!(["worker", "w"])),
        ("parent", "_activate", json!(["worker", "w", 1, "t", null])),
        ("worker", "send", json!(["one", "parent", "Please review"])),
    ] {
        call(&handles, member, method, args).unwrap();
    }
    let batch = call(&handles, "worker", "_notifications", json!([true])).unwrap();
    let members: Vec<WireMember> = serde_json::from_value(batch["members"].clone()).unwrap();
    let decoded: Vec<Member> = members
        .into_iter()
        .map(decode_member)
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(decoded.len(), 1, "{batch}");
    assert_eq!(decoded[0].id, "parent");
    assert!(batch["generation"].as_u64().is_some(), "{batch}");
    call(
        &handles,
        "worker",
        "send",
        json!(["two", "parent", "Again"]),
    )
    .unwrap();
    let list = call(&handles, "worker", "_notifications", json!([])).unwrap();
    let members: Vec<WireMember> = serde_json::from_value(list.clone()).unwrap();
    let decoded = members
        .into_iter()
        .map(decode_member)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(decoded.len(), 1, "{list}");
    assert_eq!(decoded[0].id, "parent");
}

fn panicking_board(
    _: crate::application::swarm::dto::BoardLocation,
    _: Option<std::sync::Arc<dyn crate::application::swarm::ports::BoardOpLog>>,
) -> crate::infrastructure::tools::swarm_board_dispatch::SwarmBoardHandles {
    panic!("the accounting job panicked")
}

fn observation() -> crate::domain::inference::events::request_observation::RequestObservation {
    serde_json::from_value(json!({
        "request_id": "r1", "model": "m", "provider": "p", "outcome": "ok",
        "error_class": null, "input_tokens": null, "context_input_tokens": null,
        "output_tokens": null, "cache_read_tokens": null, "cache_write_tokens": null,
        "estimated_cost_micro_usd": null, "estimated_context_tokens": 0,
        "instrumented_attempts": 1, "oauth_retries": 0, "duration_ms": 1,
        "harness_prefix_sha256": "", "harness_prefix_bytes": 0,
        "harness_prefix_unchanged": null
    }))
    .unwrap()
}

/// A request's accounting job that panics is a durable accounting error
/// the agent loop logs and drops (#2278 final review L3), never a panic
/// resumed in the loop.
#[tokio::test]
async fn a_panicking_accounting_job_is_an_error_not_a_panic_in_the_loop() {
    use crate::application::providers::ports::RequestAccounting;
    let directory = tempfile::tempdir().unwrap();
    let context = SwarmContext {
        board: crate::infrastructure::tools::swarm_bridge::SwarmBoard::new(
            panicking_board,
            crate::composition::swarm::board_wire(),
        ),
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
    };
    let recorded = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(
        context.record(&observation()),
    ))
    .await
    .expect("the accounting job's panic stays in its job");
    let error = recorded.unwrap_err();
    assert!(error.to_string().contains("panicked"), "{error}");
    // Not the store's contention (`is locked`), which the loop retries: a
    // durable rejection, which it logs and drops.
    assert!(!error.to_string().contains("is locked"), "{error}");
}

fn coordinator_status(coordinator: &str, status: &str, idle: i64) -> serde_json::Value {
    serde_json::json!({
        "members_without_claim": idle,
        "members_dead": 0,
        "id": "run-1",
        "status": status,
        "deadline": 1.0,
        "coordinator": coordinator,
        "outcome": null,
    })
}

fn totals(ready: i64, claimed: i64, submitted: i64) -> serde_json::Value {
    serde_json::json!({"run_id": "run-1", "tasks": {
        "total": ready + claimed + submitted, "ready": ready, "claimed": claimed,
        "blocked": 0, "submitted": submitted, "completed": 0}})
}

fn read_board(
    member: &str,
    status: &serde_json::Value,
    totals: serde_json::Value,
) -> Result<Option<crate::domain::swarm::parent_wake::CoordinatorBoard>, DomainError> {
    decode_coordinator_board(member, status, || Ok(totals))
}

/// #2467: the coordinator reads its run's status and outcome, its ready,
/// claimed and submitted tasks and its free workers from `_status` and
/// `_run_totals`.
#[test]
fn the_coordinator_reads_its_board_from_status_and_totals() {
    let board = read_board(
        "member-2",
        &coordinator_status("member-2", "running", 1),
        totals(2, 3, 4),
    )
    .unwrap();
    assert_eq!(
        board,
        Some(crate::domain::swarm::parent_wake::CoordinatorBoard {
            status: RunStatus::Running,
            outcome: None,
            ready: 2,
            claimed: 3,
            submitted: 4,
            idle_workers: 1,
        })
    );
}

/// #2467: a run the board ended is `paused` holding its outcome.
#[test]
fn the_coordinator_reads_an_ended_run_s_outcome() {
    let mut status = coordinator_status("member-2", "paused", 0);
    status["outcome"] = serde_json::json!("succeeded");
    let board = read_board("member-2", &status, totals(0, 0, 0))
        .unwrap()
        .expect("the coordinator reads its board");
    assert_eq!(board.status, RunStatus::Paused);
    assert_eq!(board.outcome, Some(RunStatus::Succeeded));
}

/// #2467: a member that is not the run's coordinator reads nothing, and
/// never reads the run's totals.
#[test]
fn a_worker_reads_no_coordinator_board() {
    let board = decode_coordinator_board(
        "member-3",
        &coordinator_status("member-2", "running", 1),
        || panic!("a worker must not read the run's totals"),
    )
    .unwrap();
    assert_eq!(board, None);
}

/// #2467: answers missing or mistyping the fields the decision needs are
/// an error, so the parent falls back to its ordinary turn-end note.
#[test]
fn a_malformed_answer_is_an_error() {
    let status = coordinator_status("member-2", "running", 1);
    let missing_counts = serde_json::json!({"run_id": "run-1"});
    assert!(read_board("member-2", &status, missing_counts).is_err());
    let missing_status = serde_json::json!({"coordinator": "member-2"});
    assert!(read_board("member-2", &missing_status, totals(0, 0, 0)).is_err());
    let mut numeric_status = status.clone();
    numeric_status["status"] = serde_json::json!(3);
    assert!(read_board("member-2", &numeric_status, totals(0, 0, 0)).is_err());
    let mut unknown_status = status.clone();
    unknown_status["status"] = serde_json::json!("complete");
    assert!(read_board("member-2", &unknown_status, totals(0, 0, 0)).is_err());
    let mut null_coordinator = status.clone();
    null_coordinator["coordinator"] = serde_json::Value::Null;
    assert!(read_board("member-2", &null_coordinator, totals(0, 0, 0)).is_err());
    let mut numeric_outcome = status.clone();
    numeric_outcome["outcome"] = serde_json::json!(1);
    assert!(read_board("member-2", &numeric_outcome, totals(0, 0, 0)).is_err());
    let unreadable_totals = decode_coordinator_board("member-2", &status, || {
        Err(DomainError::Tool("database is locked".into()))
    });
    assert!(unreadable_totals.is_err());
}

/// #2471: the owner of each claimed task, one per task; a claimed task
/// with no owner, a task with no status, or no list is an error.
#[test]
fn claimed_owners_are_read_from_tasks() {
    let tasks = serde_json::json!([
        {"id": 1, "status": "claimed", "owner": "m-a"},
        {"id": 2, "status": "ready", "owner": null},
        {"id": 3, "status": "claimed", "owner": "m-a"},
        {"id": 4, "status": "completed", "owner": "m-b"},
    ]);
    assert_eq!(
        decode_claimed_owners(&tasks).unwrap(),
        vec!["m-a".to_owned(), "m-a".to_owned()]
    );
    for malformed in [
        serde_json::json!([{"status": "claimed", "owner": null}]),
        serde_json::json!([{"owner": "m-a"}]),
        serde_json::json!({"tasks": []}),
    ] {
        assert!(decode_claimed_owners(&malformed).is_err(), "{malformed}");
    }
}
