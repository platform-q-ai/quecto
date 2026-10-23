//! SPIKE (board service): the measurement tools: latency, the change
//! feed's delay and resume, `kill -9` durability with idempotent retries,
//! an online backup, and contention in the server's own process.
use super::client::ServiceBoardStore;
use super::files::Head;
use super::http::{self, Endpoint};
use super::sqlite::SqliteBoard;
use super::tools::{flag, number};
use super::wire::FeedEvent;
use crate::application::project_board::ports::{BoardWrite, ProjectBoardStore};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let at = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[at.min(sorted.len() - 1)]
}

fn summary(mut samples: Vec<f64>) -> serde_json::Value {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    serde_json::json!({
        "n": samples.len(),
        "p50_ms": percentile(&samples, 0.5),
        "p95_ms": percentile(&samples, 0.95),
        "max_ms": samples.last().copied().unwrap_or(f64::NAN),
    })
}

fn identity(name: &str) -> Identity {
    Identity {
        name: name.into(),
        email: format!("{name}@quecto.invalid"),
    }
}

/// A task to write: `id`, ready, titled `title`.
pub fn bench_task(id: &str, title: &str) -> Task {
    let at = Timestamp::parse("2026-10-23T08:00:00Z").expect("a timestamp");
    Task::new(TaskFields {
        id: Slug::parse(id).expect("a slug"),
        title: title.into(),
        kind: TaskKind::Task,
        description: String::new(),
        status: TaskStatus::Draft,
        parent: None,
        depends_on: vec![],
        accepted_plan: None,
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: at.clone(),
        updated: at,
    })
    .expect("a task")
}

/// `bench-latency --endpoint <url> [--n 200]`: the in-process cost of a
/// read (snapshot fetched and decoded) and a write (one task), no process
/// start.
pub(super) fn bench_latency(args: &[String]) -> Result<(), String> {
    let endpoint = flag(args, "--endpoint").ok_or("--endpoint")?;
    let n = number(args, "--n", 200);
    let store = ServiceBoardStore::new(&endpoint, identity("bench"))?;
    let (mut reads, mut writes) = (vec![], vec![]);
    let task = bench_task("bench-latency", "latency probe");
    for _ in 0..n {
        let started = Instant::now();
        let board = store.read().map_err(|error| format!("{error:?}"))?;
        reads.push(started.elapsed().as_secs_f64() * 1000.0);
        let started = Instant::now();
        let written = store
            .write_tasks(
                &board.head,
                std::slice::from_ref(&task),
                "board: latency probe",
            )
            .map_err(|error| format!("{error:?}"))?;
        writes.push(started.elapsed().as_secs_f64() * 1000.0);
        if !matches!(written, BoardWrite::Written(_)) {
            return Err(format!("not written: {written:?}"));
        }
    }
    println!(
        "{}",
        serde_json::json!({ "read": summary(reads), "write": summary(writes) })
    );
    Ok(())
}

/// `bench-feed --endpoint <url> [--n 100]`: the delay from a write's
/// answer to its event on the feed, and a resume after a break with no
/// revision missed or repeated.
pub(super) fn bench_feed(args: &[String]) -> Result<(), String> {
    let endpoint = flag(args, "--endpoint").ok_or("--endpoint")?;
    let feed_endpoint =
        Endpoint::parse(endpoint.split(',').next().unwrap_or("")).ok_or("an endpoint")?;
    let n = number(args, "--n", 100);
    // `--break-ms 0`: no break, only the steady delay is measured.
    let break_ms = number(args, "--break-ms", 300);
    let store = ServiceBoardStore::new(&endpoint, identity("bench"))?;
    let start = store.read().map_err(|error| format!("{error:?}"))?;
    let from: u64 = start
        .head
        .0
        .split(':')
        .nth(1)
        .and_then(|r| r.parse().ok())
        .ok_or("a head")?;
    let received: Arc<Mutex<Vec<(u64, Instant)>>> = Arc::default();
    let breaks = Arc::new(Mutex::new(0u32));
    let listener = {
        let received = Arc::clone(&received);
        let breaks = Arc::clone(&breaks);
        let feed_endpoint = feed_endpoint.clone();
        std::thread::spawn(move || {
            let mut last = from;
            let target = from + n;
            while last < target {
                let mut seen_here = 0;
                let _ = http::events(
                    &feed_endpoint,
                    &format!("/feed?from={last}"),
                    Duration::from_secs(30),
                    |event, data| {
                        if event != "revision" {
                            return true;
                        }
                        let Ok(event) = serde_json::from_str::<FeedEvent>(data) else {
                            return false;
                        };
                        received
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push((event.revision, Instant::now()));
                        last = event.revision;
                        seen_here += 1;
                        // Break the connection once, half way, to resume.
                        let first_break =
                            break_ms > 0 && *breaks.lock().unwrap_or_else(|e| e.into_inner()) == 0;
                        !(first_break && last >= from + n / 2 && seen_here > 0) && last < target
                    },
                );
                let mut broke = breaks.lock().unwrap_or_else(|e| e.into_inner());
                if *broke == 0 && last < target {
                    *broke = 1;
                    drop(broke);
                    // Writes go on while the subscriber is away.
                    std::thread::sleep(Duration::from_millis(break_ms));
                }
            }
        })
    };
    std::thread::sleep(Duration::from_millis(200));
    let task = bench_task("bench-feed", "feed probe");
    let mut acked: BTreeMap<u64, (Instant, Instant)> = BTreeMap::new();
    let mut head = start.head;
    for _ in 0..n {
        let sent = Instant::now();
        match store.write_tasks(&head, std::slice::from_ref(&task), "board: feed probe") {
            Ok(BoardWrite::Written(moved)) => {
                let revision: u64 = moved
                    .0
                    .split(':')
                    .nth(1)
                    .and_then(|r| r.parse().ok())
                    .ok_or("a head")?;
                acked.insert(revision, (sent, Instant::now()));
                head = moved;
            }
            other => return Err(format!("feed write: {other:?}")),
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    listener.join().map_err(|_| "the listener panicked")?;
    let received = received.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let revisions: Vec<u64> = received.iter().map(|(r, _)| *r).collect();
    let expected: Vec<u64> = (from + 1..=from + n).collect();
    let (mut after_ack, mut after_send) = (vec![], vec![]);
    for (revision, at) in &received {
        if let Some((sent, ack)) = acked.get(revision) {
            after_send.push(at.saturating_duration_since(*sent).as_secs_f64() * 1000.0);
            let signed = match at.checked_duration_since(*ack) {
                Some(after) => after.as_secs_f64() * 1000.0,
                None => -(ack.saturating_duration_since(*at).as_secs_f64() * 1000.0),
            };
            after_ack.push(signed);
        }
    }
    println!(
        "{}",
        serde_json::json!({
            "events": revisions.len(),
            "gap_free_and_once_each": revisions == expected,
            "resumed_after_break": *breaks.lock().unwrap_or_else(|e| e.into_inner()) == 1,
            "event_after_write_ack": summary(after_ack),
            "event_after_write_start": summary(after_send),
        })
    );
    Ok(())
}

/// `kill-test --exe <quecto> --dir <scratch> [--writers 4] [--secs 3]
/// [--drop-every 5]`: a server child killed with SIGKILL while writers
/// write; restarted; every acknowledged write must be on the board, and
/// no write applied twice (retries after a dropped answer included).
pub(super) fn kill_test(args: &[String]) -> Result<(), String> {
    let exe = flag(args, "--exe").ok_or("--exe")?;
    let dir = PathBuf::from(flag(args, "--dir").ok_or("--dir")?);
    let writers = number(args, "--writers", 4);
    let secs = number(args, "--secs", 3);
    let drop_every = number(args, "--drop-every", 5);
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let db = dir.join("kill.sqlite");
    let socket = dir.join("kill.sock");
    let spawn = || {
        Command::new(&exe)
            .args(["board-server", "--db"])
            .arg(&db)
            .arg("--socket")
            .arg(&socket)
            .env("QUECTO_SPIKE_DROP_EVERY", drop_every.to_string())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| error.to_string())
    };
    let wait_up = || {
        for _ in 0..100 {
            if http::call(
                &Endpoint::Unix(socket.clone()),
                "GET",
                "/status",
                None,
                Duration::from_secs(1),
            )
            .is_ok()
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    };
    let mut child = spawn()?;
    if !wait_up() {
        return Err("the server did not come up".into());
    }
    let spec = format!("unix:{}", socket.display());
    let boot = ServiceBoardStore::new(&spec, identity("kill"))?;
    boot.bootstrap().map_err(|error| format!("{error:?}"))?;
    let acked: Arc<Mutex<Vec<String>>> = Arc::default();
    let errors: Arc<Mutex<Vec<String>>> = Arc::default();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut threads = vec![];
    for w in 0..writers {
        let (spec, acked, errors, stop) = (
            spec.clone(),
            Arc::clone(&acked),
            Arc::clone(&errors),
            Arc::clone(&stop),
        );
        threads.push(std::thread::spawn(move || {
            let Ok(store) = ServiceBoardStore::new(&spec, identity(&format!("w{w}"))) else {
                return;
            };
            let mut n = 0;
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                let id = format!("k-w{w}-{n}");
                let Ok(board) = store.read() else {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                };
                match store.write_tasks(
                    &board.head,
                    &[bench_task(&id, "kill probe")],
                    "board: kill probe",
                ) {
                    Ok(BoardWrite::Written(_)) => {
                        acked.lock().unwrap_or_else(|e| e.into_inner()).push(id);
                        n += 1;
                    }
                    Ok(_) => {}
                    Err(error) => {
                        errors
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(format!("{error:?}"));
                        // An answer lost in the outage: the write may have
                        // landed. Look before writing it again.
                        if let Ok(board) = store.read() {
                            if board
                                .tasks
                                .iter()
                                .any(|task| task.fields().id.as_str() == id)
                            {
                                n += 1;
                            }
                        }
                    }
                }
            }
        }));
    }
    std::thread::sleep(Duration::from_secs(secs));
    child.kill().map_err(|error| error.to_string())?;
    let _ = child.wait();
    let killed_at = Instant::now();
    std::thread::sleep(Duration::from_millis(300));
    let mut child = spawn()?;
    let up = wait_up();
    let restart_ms = killed_at.elapsed().as_secs_f64() * 1000.0;
    std::thread::sleep(Duration::from_secs(1));
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    for thread in threads {
        let _ = thread.join();
    }
    let board = boot.read().map_err(|error| format!("{error:?}"))?;
    let on_board: BTreeSet<String> = board
        .tasks
        .iter()
        .map(|task| task.fields().id.as_str().to_string())
        .collect();
    let acked = acked.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let missing: Vec<&String> = acked.iter().filter(|id| !on_board.contains(*id)).collect();
    // Each task was written once by its writer: a write applied twice would
    // be a second revision naming the same task.
    let opened = SqliteBoard::open(&db).map_err(|error| format!("{error:?}"))?;
    let rows = opened
        .revisions_after(0, u64::MAX >> 1)
        .map_err(|error| format!("{error:?}"))?;
    let mut per_task: BTreeMap<String, u64> = BTreeMap::new();
    let mut write_ids = BTreeSet::new();
    let mut duplicate_ids = 0;
    for row in &rows {
        if let Some(id) = &row.write_id {
            if !write_ids.insert(id.clone()) {
                duplicate_ids += 1;
            }
        }
        if row.kind == "write" {
            let (_, changes) = opened
                .revision(row.revision)
                .map_err(|error| format!("{error:?}"))?;
            for change in changes.iter().filter(|c| c.path.starts_with("tasks/")) {
                *per_task.entry(change.path.clone()).or_default() += 1;
            }
        }
    }
    let twice: Vec<_> = per_task.iter().filter(|(_, n)| **n > 1).collect();
    let integrity: String = Command::new("sqlite3")
        .arg(&db)
        .arg("PRAGMA integrity_check;")
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_else(|error| error.to_string());
    let _ = child.kill();
    let _ = child.wait();
    let errors = errors.lock().unwrap_or_else(|e| e.into_inner()).clone();
    println!(
        "{}",
        serde_json::json!({
            "acknowledged_writes": acked.len(),
            "acknowledged_missing": missing.len(),
            "tasks_written_twice": twice.len(),
            "duplicate_write_ids": duplicate_ids,
            "revisions": rows.len(),
            "server_back_up": up,
            "restart_ms": restart_ms,
            "client_errors_during_outage": errors.len(),
            "first_errors": errors.iter().take(3).collect::<Vec<_>>(),
            "integrity_check": integrity,
        })
    );
    Ok(())
}

/// `backup --db <file> --to <file>`: SQLite's online backup, timed, and
/// checked.
pub(super) fn backup(args: &[String]) -> Result<(), String> {
    let db = PathBuf::from(flag(args, "--db").ok_or("--db")?);
    let to = PathBuf::from(flag(args, "--to").ok_or("--to")?);
    let board = SqliteBoard::open(&db).map_err(|error| format!("{error:?}"))?;
    let started = Instant::now();
    board.backup(&to).map_err(|error| format!("{error:?}"))?;
    let took = started.elapsed();
    let copy = SqliteBoard::open(&to).map_err(|error| format!("{error:?}"))?;
    let reader = BufReader::new(
        Command::new("sqlite3")
            .arg(&to)
            .arg("PRAGMA integrity_check;")
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| error.to_string())?
            .stdout
            .ok_or("no stdout")?,
    );
    let integrity: Vec<String> = reader.lines().map_while(Result::ok).collect();
    println!(
        "{}",
        serde_json::json!({
            "backup_ms": took.as_secs_f64() * 1000.0,
            "head_live": board.head().ok().flatten().map(Head::text),
            "head_copy": copy.head().ok().flatten().map(Head::text),
            "integrity": integrity,
        })
    );
    Ok(())
}

/// `bench-contention-local --db <file> [--writers 8] [--writes 50]`: the
/// writers run the create use case in the server's own process, over the
/// SQLite store (what board operations run server-side would see): no
/// network and no client decode between a read and its write.
pub(super) fn bench_contention_local(args: &[String]) -> Result<(), String> {
    use crate::application::project_board::dto::{
        NewBoardTask, NewBoardTaskStatus, ProjectBoardError,
    };
    use crate::application::project_board::use_cases::CreateBoardTask;
    use crate::infrastructure::time::board_clock::SystemBoardClock;
    let db = PathBuf::from(flag(args, "--db").ok_or("--db")?);
    let writers = number(args, "--writers", 8);
    let writes = number(args, "--writes", 50);
    let board = Arc::new(SqliteBoard::open(&db).map_err(|error| format!("{error:?}"))?);
    let started = Instant::now();
    let mut threads = vec![];
    for w in 0..writers {
        let board = Arc::clone(&board);
        threads.push(std::thread::spawn(move || {
            let store: Arc<dyn ProjectBoardStore> = Arc::new(super::store::SqliteBoardStore::new(
                board,
                identity(&format!("local{w}")),
            ));
            let create = CreateBoardTask::new(store, Arc::new(SystemBoardClock::default()))
                .expect("a use case");
            let (mut ok, mut contended, mut other, mut lat) = (0u64, 0u64, 0u64, vec![]);
            for i in 0..writes {
                let new = NewBoardTask {
                    id: Slug::parse(&format!("l-{w}-{i}")).expect("a slug"),
                    title: format!("local {w} {i}"),
                    kind: TaskKind::Task,
                    description: String::new(),
                    status: NewBoardTaskStatus::Ready,
                    parent: None,
                    depends_on: vec![],
                };
                let at = Instant::now();
                match create.execute(&new) {
                    Ok(_) => ok += 1,
                    Err(ProjectBoardError::Contended { .. }) => contended += 1,
                    Err(_) => other += 1,
                }
                lat.push(at.elapsed().as_secs_f64() * 1000.0);
            }
            (ok, contended, other, lat)
        }));
    }
    let (mut ok, mut contended, mut other, mut lat) = (0, 0, 0, vec![]);
    for thread in threads {
        let (o, c, x, l) = thread.join().map_err(|_| "a writer panicked")?;
        ok += o;
        contended += c;
        other += x;
        lat.extend(l);
    }
    let wall = started.elapsed().as_secs_f64();
    let on_board = board
        .current()
        .map_err(|error| format!("{error:?}"))?
        .snapshot
        .tasks
        .iter()
        .filter(|task| task.fields().id.as_str().starts_with("l-"))
        .count();
    println!(
        "{}",
        serde_json::json!({
            "attempted": writers * writes,
            "ok": ok,
            "contended": contended,
            "other_errors": other,
            "lost_race_rate": contended as f64 / (writers * writes) as f64,
            "on_board": on_board,
            "throughput_per_s": ok as f64 / wall,
            "latency": summary(lat),
        })
    );
    Ok(())
}
