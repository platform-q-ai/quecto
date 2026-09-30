//! The ports the run summary added (#2313), on their production adapters:
//! `BoardMessages::message_tally`, `BoardEvents::created_at` and
//! `BoardUsage::member_usage` on SQLite (a creation time the board did not
//! write is none; the usage read creates no table), the metered call's
//! `caller_authorized` and `role_fixed`, and the event log's `summarize`
//! and its `SessionOpLog` drop notes and hand-over.
use quecto::application::swarm::dto::{BoardLocation, DroppedRecords, MessageTally};
use quecto::application::swarm::ports::{
    BoardCallMeter, BoardOpLog, BoardRepository, SessionOpLog,
};
use quecto::domain::swarm::RunSummaryFold;
use quecto::infrastructure::persistence::audit_log::AuditLog;
use quecto::infrastructure::persistence::board_op_log::EventLogBoardOps;
use quecto::infrastructure::persistence::swarm_board::meter::SqliteBoardCallMeter;
use quecto::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;
use serde_json::json;

fn board() -> (tempfile::TempDir, std::path::PathBuf, SqliteBoardRepository) {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let repository = SqliteBoardRepository::new(&BoardLocation {
        database: database.clone(),
        checkout: dir.path().to_path_buf(),
    });
    repository.atomic(true, &mut |_| Ok(())).unwrap();
    (dir, database, repository)
}

fn execute(database: &std::path::Path, sql: &str) {
    rusqlite::Connection::open(database)
        .unwrap()
        .execute_batch(sql)
        .unwrap();
}

/// Every row counts as sent; `consumed` and `withdrawn` by their status.
#[test]
fn the_message_tally_counts_rows_by_status() {
    let (_dir, database, repository) = board();
    let tally = |repository: &SqliteBoardRepository| {
        let mut tally = None;
        repository
            .atomic(false, &mut |transaction| {
                tally = Some(transaction.message_tally()?);
                Ok(())
            })
            .unwrap();
        tally.unwrap()
    };
    assert_eq!(tally(&repository), MessageTally::default());
    execute(
        &database,
        "INSERT INTO messages(sender,recipient,body,status) VALUES
           ('w','p','a','accepted'),('w','p','b','consumed'),
           ('w','p','c','withdrawn'),('w','p','d',NULL)",
    );
    assert_eq!(
        tally(&repository),
        MessageTally {
            sent: 4,
            consumed: 1,
            withdrawn: 1,
        }
    );
}

/// The latest `created` event's time: a REAL or an integer is a time; a
/// time stored as text or NULL, or no `created` event, is none.
#[test]
fn the_creation_time_is_the_latest_created_events_number() {
    let (_dir, database, repository) = board();
    let created = |repository: &SqliteBoardRepository| {
        let mut created = None;
        repository
            .atomic(false, &mut |transaction| {
                created = Some(transaction.created_at()?);
                Ok(())
            })
            .unwrap();
        created.unwrap()
    };
    assert_eq!(created(&repository), None, "no created event");
    repository
        .atomic(false, &mut |transaction| {
            transaction.event("parent", 12.5, "created", &json!({}))?;
            transaction.event("parent", 20.0, "claimed", &json!({}))
        })
        .unwrap();
    assert_eq!(created(&repository), Some(12.5));
    execute(
        &database,
        "INSERT INTO events(actor,time,action,detail) VALUES('p',30,'created','{}')",
    );
    assert_eq!(created(&repository), Some(30.0), "the latest, an integer");
    execute(
        &database,
        "INSERT INTO events(actor,time,action,detail) VALUES('p','soon','created','{}')",
    );
    assert_eq!(created(&repository), None, "text is no time");
    execute(
        &database,
        "INSERT INTO events(actor,time,action,detail) VALUES('p',NULL,'created','{}')",
    );
    assert_eq!(created(&repository), None, "NULL is no time");
}

/// The per-member usage reads no payload and creates no table: none on a
/// board without the ledger, the aggregates once it has one.
#[test]
fn the_member_usage_creates_no_table_and_reads_no_payload() {
    let (_dir, database, repository) = board();
    let usage = |repository: &SqliteBoardRepository| {
        let mut usage = None;
        repository
            .atomic(false, &mut |transaction| {
                usage = Some(transaction.member_usage()?);
                Ok(())
            })
            .unwrap();
        usage.unwrap()
    };
    assert!(usage(&repository).is_empty());
    let tables: i64 = rusqlite::Connection::open(&database)
        .unwrap()
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='request_usage'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 0, "no table created");
    execute(
        &database,
        "CREATE TABLE request_usage (request_id TEXT PRIMARY KEY, actor TEXT, payload TEXT, tokens INTEGER, unknown INTEGER, attempts INTEGER, input_tokens INTEGER, output_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER);
         INSERT INTO request_usage VALUES('r1','w','{not json',7,0,1,5,2,NULL,NULL);",
    );
    let rows = usage(&repository);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("member"), Some(&json!("w")));
    assert_eq!(rows[0].get("requests"), Some(&json!(1)));
    assert_eq!(rows[0].get("reported_input_tokens"), Some(&json!(5)));
}

/// A run row for the metered calls to find.
fn with_run(database: &std::path::Path) {
    execute(
        database,
        "INSERT INTO run(id,goal,constraints,criteria,coordinator,integrator,member_limit,deadline,status)
         VALUES('0123456789abcdef0123456789abcdef','','[]','[]','parent','parent',3,0,'setup')",
    );
}

/// `caller_authorized` marks the metered call's measure; a call that was
/// not authorised says so; the unmetered repository measures nothing.
#[test]
fn an_authorised_caller_is_noted_on_the_metered_call_only() {
    let (_dir, _database, repository) = board();
    let meter = SqliteBoardCallMeter::new(repository.clone());
    let call = meter.open();
    call.atomic(false, &mut |transaction| {
        transaction.caller_authorized();
        Ok(())
    })
    .unwrap();
    assert!(call.measure().unwrap().authorized);
    let other = meter.open();
    other.atomic(false, &mut |_| Ok(())).unwrap();
    assert!(!other.measure().unwrap().authorized);
    repository
        .atomic(false, &mut |transaction| {
            transaction.caller_authorized();
            Ok(())
        })
        .unwrap();
}

/// `role_fixed`: once a call whose role is fixed has found the run's id
/// (here through `run_status`, which names no integrator), it runs no
/// statement of its own for the roles; a call whose role is not fixed
/// reads them.
#[test]
fn a_call_whose_role_is_fixed_reads_no_roles_once_it_has_the_run_id() {
    let (_dir, database, repository) = board();
    with_run(&database);
    let meter = SqliteBoardCallMeter::new(repository);
    let status = |call: &dyn quecto::application::swarm::ports::MeteredCall| {
        call.atomic(false, &mut |transaction| {
            transaction.run_status().map(|_| ())
        })
        .unwrap();
        call.measure().unwrap()
    };
    let fixed = meter.open();
    fixed.role_fixed();
    let measure = status(&*fixed);
    assert!(measure.run_id.is_some(), "{measure:?}");
    assert_eq!(measure.run_roles, None, "{measure:?}");
    let open = meter.open();
    assert!(status(&*open).run_roles.is_some());
}

fn lines(base: &std::path::Path, key: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(AuditLog::file_path(base, key))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// The event log writes a run summary as one line under no turn, and its
/// drop notes: one line per kind dropped, none for nothing.
#[test]
fn the_event_log_writes_the_summary_and_the_drop_notes() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:summary").unwrap();
    let ops = EventLogBoardOps::new(log.crash_line().unwrap());
    let run = "0123456789abcdef0123456789abcdef";
    ops.summarize(RunSummaryFold::new(run, 0).summary(1, None));
    ops.dropped(DroppedRecords::default());
    ops.dropped(DroppedRecords {
        ops: 2,
        summaries: 1,
    });
    let written = lines(base.path(), "cli:summary");
    assert_eq!(written.len(), 3, "{written:?}");
    assert_eq!(written[0]["event"], "swarm_run_summary");
    assert_eq!(written[0]["turn"], serde_json::Value::Null);
    assert_eq!(
        (&written[1]["event"], &written[1]["dropped"]),
        (&json!("swarm_ops_dropped"), &json!(2))
    );
    assert_eq!(
        (&written[2]["event"], &written[2]["dropped"]),
        (&json!("swarm_run_summary_dropped"), &json!(1))
    );
}

/// A log that noted its drops holds none unnoted: nothing is handed to
/// the log that replaces it (what the gate held off is handed over, as
/// the adapter's own tests pin with the gate held).
#[test]
fn noted_drops_are_not_handed_over() {
    let base = tempfile::tempdir().unwrap();
    let log = AuditLog::open_sync(base.path(), "cli:noted").unwrap();
    let ops = EventLogBoardOps::new(log.crash_line().unwrap());
    assert_eq!(ops.take_unnoted(), DroppedRecords::default(), "fresh");
    ops.dropped(DroppedRecords {
        ops: 1,
        summaries: 1,
    });
    assert_eq!(ops.take_unnoted(), DroppedRecords::default(), "noted");
    assert_eq!(lines(base.path(), "cli:noted").len(), 2);
}
