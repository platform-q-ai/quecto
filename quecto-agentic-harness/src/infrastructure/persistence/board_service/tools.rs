//! SPIKE (board service): the importer from a `quecto/board` branch (full
//! history, every commit a revision), the export back to git (each commit
//! rebuilt byte for byte from the database), and the measurement tools.
use super::client::ServiceBoardStore;
use super::files::{BoardFile, Head};
use super::http::{self, Endpoint};
use super::sqlite::{FileChange, RevisionMeta, SqliteBoard};
use super::wire::FeedEvent;
use crate::application::project_board::ports::{BoardWrite, ProjectBoardStore};
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{TaskKind, TaskStatus};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|at| args.get(at + 1))
        .cloned()
}

fn number(args: &[String], name: &str, default: u64) -> u64 {
    flag(args, name)
        .and_then(|n| n.parse().ok())
        .unwrap_or(default)
}

pub fn run(args: &[String]) -> i32 {
    let outcome = match args.first().map(String::as_str) {
        Some("import") => import(args),
        Some("export") => export(args),
        Some("bench-latency") => bench_latency(args),
        Some("bench-feed") => bench_feed(args),
        Some("kill-test") => kill_test(args),
        Some("backup") => backup(args),
        Some("bench-contention-local") => bench_contention_local(args),
        _ => Err("unknown tool".into()),
    };
    match outcome {
        Ok(()) => 0,
        Err(why) => {
            eprintln!("board-server: {why}");
            1
        }
    }
}

fn git(dir: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let mut child = Command::new("git")
        .arg("--git-dir")
        .arg(dir)
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    if let (Some(bytes), Some(mut stdin)) = (input, child.stdin.take()) {
        let bytes = bytes.to_vec();
        std::thread::spawn(move || stdin.write_all(&bytes));
    }
    let out = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    match out.status.success() {
        true => Ok(out.stdout),
        false => Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        )),
    }
}

fn text(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).trim().to_string()
}

/// Reads blobs through one `cat-file --batch`.
struct Blobs {
    child: std::process::Child,
    cache: HashMap<String, Vec<u8>>,
}

impl Blobs {
    fn new(dir: &Path) -> Result<Self, String> {
        let child = Command::new("git")
            .arg("--git-dir")
            .arg(dir)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            child,
            cache: HashMap::new(),
        })
    }

    fn get(&mut self, oid: &str) -> Result<Vec<u8>, String> {
        if let Some(bytes) = self.cache.get(oid) {
            return Ok(bytes.clone());
        }
        let stdin = self.child.stdin.as_mut().ok_or("no stdin")?;
        writeln!(stdin, "{oid}").map_err(|error| error.to_string())?;
        stdin.flush().map_err(|error| error.to_string())?;
        let stdout = self.child.stdout.as_mut().ok_or("no stdout")?;
        let mut header = vec![];
        let mut byte = [0u8];
        loop {
            stdout
                .read_exact(&mut byte)
                .map_err(|error| error.to_string())?;
            if byte[0] == b'\n' {
                break;
            }
            header.push(byte[0]);
        }
        let header = String::from_utf8_lossy(&header).to_string();
        let size: usize = header
            .split(' ')
            .nth(2)
            .and_then(|size| size.parse().ok())
            .ok_or_else(|| format!("cat-file said {header:?}"))?;
        let mut bytes = vec![0; size + 1];
        stdout
            .read_exact(&mut bytes)
            .map_err(|error| error.to_string())?;
        bytes.truncate(size);
        self.cache.insert(oid.to_string(), bytes.clone());
        Ok(bytes)
    }
}

/// Mode marking an empty directory (a tree with nothing under it).
const EMPTY_DIR: &str = "040000";

/// Every file of commit `commit`, by path: blobs (and links) with their
/// bytes, a gitlink with its id as text, an empty tree as itself.
fn tree_files(
    dir: &Path,
    commit: &str,
    blobs: &mut Blobs,
) -> Result<BTreeMap<String, (String, Vec<u8>)>, String> {
    let listing = git(
        dir,
        &["ls-tree", "-r", "-t", "-z", "--full-tree", commit],
        None,
    )?;
    let mut files = BTreeMap::new();
    let mut trees = BTreeSet::new();
    for entry in listing.split(|b| *b == 0).filter(|e| !e.is_empty()) {
        let entry = String::from_utf8(entry.to_vec()).map_err(|_| "a non-UTF-8 path")?;
        let (meta, path) = entry.split_once('\t').ok_or("a malformed ls-tree row")?;
        let mut parts = meta.split(' ');
        let (mode, kind, oid) = (
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
            parts.next().unwrap_or(""),
        );
        match kind {
            "blob" => {
                files.insert(path.to_string(), (mode.to_string(), blobs.get(oid)?));
            }
            "tree" => {
                trees.insert(path.to_string());
            }
            _ => {
                files.insert(
                    path.to_string(),
                    (mode.to_string(), oid.as_bytes().to_vec()),
                );
            }
        }
    }
    for tree in trees {
        let prefix = format!("{tree}/");
        if !files.keys().any(|path| path.starts_with(&prefix)) {
            files.insert(tree, (EMPTY_DIR.to_string(), vec![]));
        }
    }
    Ok(files)
}

/// A raw commit object's header and message without its `tree` and
/// `parent` lines: what the export puts back after them.
fn commit_meta(raw: &[u8]) -> (Vec<u8>, Identity, String) {
    let split = raw
        .windows(2)
        .position(|w| w == b"\n\n")
        .map_or(raw.len(), |at| at + 1);
    let (header, message) = raw.split_at(split);
    let mut kept = vec![];
    let mut writer = Identity {
        name: "unknown".into(),
        email: "unknown@quecto.invalid".into(),
    };
    for line in header.split_inclusive(|b| *b == b'\n') {
        if line.starts_with(b"tree ") || line.starts_with(b"parent ") {
            continue;
        }
        if let Some(rest) = line.strip_prefix(b"committer ") {
            let rest = String::from_utf8_lossy(rest);
            if let Some((name, after)) = rest.split_once(" <") {
                writer = Identity {
                    name: name.to_string(),
                    email: after
                        .split_once('>')
                        .map_or("", |(email, _)| email)
                        .to_string(),
                };
            }
        }
        kept.extend_from_slice(line);
    }
    kept.extend_from_slice(message);
    let message = String::from_utf8_lossy(message);
    let subject = message
        .trim_start_matches('\n')
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(200)
        .collect();
    (kept, writer, subject)
}

/// `import --db <file> --git-dir <dir> --ref <ref>`: every commit of the
/// board branch, oldest first, as a revision of epoch 1.
fn import(args: &[String]) -> Result<(), String> {
    let started = Instant::now();
    let db = PathBuf::from(flag(args, "--db").ok_or("--db")?);
    let dir = PathBuf::from(flag(args, "--git-dir").ok_or("--git-dir")?);
    let reference = flag(args, "--ref").unwrap_or_else(|| "refs/heads/quecto/board".into());
    let board = SqliteBoard::open(&db).map_err(|error| format!("{error:?}"))?;
    if board
        .head()
        .map_err(|error| format!("{error:?}"))?
        .is_some()
    {
        return Err("the database already holds a board".into());
    }
    let listing = text(git(
        &dir,
        &[
            "rev-list",
            "--reverse",
            "--topo-order",
            "--parents",
            &reference,
        ],
        None,
    )?);
    let mut blobs = Blobs::new(&dir)?;
    let mut previous: Option<String> = None;
    let mut before: BTreeMap<String, (String, Vec<u8>)> = BTreeMap::new();
    let mut count = 0u64;
    for line in listing.lines() {
        let mut ids = line.split(' ');
        let commit = ids.next().ok_or("an empty rev-list row")?.to_string();
        let parents: Vec<&str> = ids.collect();
        match (parents.as_slice(), &previous) {
            ([], None) => {}
            ([parent], Some(previous)) if parent == previous => {}
            _ => {
                return Err(format!(
                    "{commit}: the board's history is not linear ({parents:?})"
                ));
            }
        }
        let raw = git(&dir, &["cat-file", "commit", &commit], None)?;
        let (meta, writer, subject) = commit_meta(&raw);
        let files = tree_files(&dir, &commit, &mut blobs)?;
        let mut changes = vec![];
        for (path, (mode, bytes)) in &files {
            if before.get(path) != Some(&(mode.clone(), bytes.clone())) {
                changes.push(FileChange {
                    path: path.clone(),
                    mode: mode.clone(),
                    bytes: Some(bytes.clone()),
                });
            }
        }
        for (path, (mode, _)) in &before {
            if !files.contains_key(path) {
                changes.push(FileChange {
                    path: path.clone(),
                    mode: mode.clone(),
                    bytes: None,
                });
            }
        }
        count += 1;
        let at = Head {
            epoch: 1,
            revision: count,
        };
        let revision_meta = RevisionMeta {
            git_commit: Some(commit.clone()),
            git_meta: Some(meta),
            ..RevisionMeta::by(
                writer,
                if subject.is_empty() {
                    "(no message)"
                } else {
                    &subject
                },
                "import",
            )
        };
        board
            .append_raw(at, &changes, &revision_meta)
            .map_err(|error| format!("{error:?}"))?;
        before = files;
        previous = Some(commit);
    }
    let current = board.current().map_err(|error| format!("{error:?}"))?;
    println!(
        "{}",
        serde_json::json!({
            "imported_revisions": count,
            "head": current.head.text(),
            "tip": previous,
            "tasks": current.snapshot.tasks.len(),
            "plans": current.snapshot.plans.len(),
            "runs": current.snapshot.runs.len(),
            "changes": current.snapshot.changes.len(),
            "reviews": current.snapshot.reviews.len(),
            "discussions": current.snapshot.discussions.len(),
            "set_aside": current.snapshot.quarantined.len(),
            "files": current.files.len(),
            "ms": started.elapsed().as_millis(),
        })
    );
    Ok(())
}

/// Builds the tree of `files` (nested), returning its id.
fn build_tree(
    dir: &Path,
    files: &[BoardFile],
    blob_ids: &mut HashMap<Vec<u8>, String>,
) -> Result<String, String> {
    #[derive(Default)]
    struct Node {
        files: BTreeMap<String, (String, String)>,
        dirs: BTreeMap<String, Node>,
        empty: bool,
    }
    let mut root = Node::default();
    for file in files {
        let parts: Vec<&str> = file.path.split('/').collect();
        let (name, folders) = parts.split_last().ok_or("an empty path")?;
        let mut node = &mut root;
        for folder in folders {
            node = node.dirs.entry((*folder).to_string()).or_default();
        }
        if file.mode == EMPTY_DIR {
            node.dirs.entry((*name).to_string()).or_default().empty = true;
            continue;
        }
        let (kind, oid) = match file.mode.as_str() {
            "160000" => ("commit", String::from_utf8_lossy(&file.bytes).to_string()),
            _ => {
                let oid = match blob_ids.get(&file.bytes) {
                    Some(oid) => oid.clone(),
                    None => {
                        let oid = text(git(
                            dir,
                            &["hash-object", "-w", "--stdin"],
                            Some(&file.bytes),
                        )?);
                        blob_ids.insert(file.bytes.clone(), oid.clone());
                        oid
                    }
                };
                ("blob", oid)
            }
        };
        node.files.insert(
            (*name).to_string(),
            (file.mode.clone(), format!("{kind} {oid}")),
        );
    }
    fn write(dir: &Path, node: &Node) -> Result<String, String> {
        let mut listing = String::new();
        for (name, (mode, entry)) in &node.files {
            listing.push_str(&format!("{mode} {entry}\t{name}\0"));
        }
        for (name, child) in &node.dirs {
            let oid = write(dir, child)?;
            listing.push_str(&format!("040000 tree {oid}\t{name}\0"));
        }
        let _ = node.empty;
        Ok(text(git(
            dir,
            &["mktree", "-z", "--missing"],
            Some(listing.as_bytes()),
        )?))
    }
    write(dir, &root)
}

/// `export --db <file> --git-dir <dir> --ref <out ref> [--format4]`: every
/// revision as a commit, imported ones rebuilt from their recorded header,
/// and how many come out as the very commit they were.
fn export(args: &[String]) -> Result<(), String> {
    let started = Instant::now();
    let db = PathBuf::from(flag(args, "--db").ok_or("--db")?);
    let dir = PathBuf::from(flag(args, "--git-dir").ok_or("--git-dir")?);
    let reference = flag(args, "--ref").ok_or("--ref")?;
    let board = SqliteBoard::open(&db).map_err(|error| format!("{error:?}"))?;
    let head = board
        .head()
        .map_err(|error| format!("{error:?}"))?
        .ok_or("no board")?;
    let mut blob_ids = HashMap::new();
    let mut parent: Option<String> = None;
    let (mut same, mut differ, mut native) = (0u64, vec![], 0u64);
    for revision in 1..=head.revision {
        let (row, _) = board
            .revision(revision)
            .map_err(|error| format!("{error:?}"))?;
        let mut files = board
            .files_at(revision)
            .map_err(|error| format!("{error:?}"))?;
        if args.iter().any(|arg| arg == "--format4") && revision == head.revision {
            for file in &mut files {
                if file.path == super::files::format_path() {
                    file.bytes = b"quecto-board 4\n".to_vec();
                }
            }
        }
        let tree = build_tree(&dir, &files, &mut blob_ids)?;
        let mut raw = format!("tree {tree}\n").into_bytes();
        if let Some(parent) = &parent {
            raw.extend_from_slice(format!("parent {parent}\n").as_bytes());
        }
        match &row.git_meta {
            Some(meta) => raw.extend_from_slice(meta),
            None => {
                native += 1;
                let when = humantime::parse_rfc3339_weak(&row.server_time)
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |since| since.as_secs());
                let who = format!("{} <{}> {when} +0000", row.writer_name, row.writer_email);
                raw.extend_from_slice(
                    format!("author {who}\ncommitter {who}\n\n{}\n", row.message).as_bytes(),
                );
            }
        }
        let commit = text(git(
            &dir,
            &["hash-object", "-t", "commit", "-w", "--stdin"],
            Some(&raw),
        )?);
        match &row.git_commit {
            Some(original) if *original == commit => same += 1,
            Some(original) => differ.push(format!("r{revision}: {original} -> {commit}")),
            None => {}
        }
        parent = Some(commit);
    }
    let tip = parent.ok_or("no revision")?;
    git(&dir, &["update-ref", &reference, &tip], None)?;
    println!(
        "{}",
        serde_json::json!({
            "exported_revisions": head.revision,
            "identical_commits": same,
            "differing_commits": differ,
            "native_revisions": native,
            "tip": tip,
            "ms": started.elapsed().as_millis(),
        })
    );
    Ok(())
}

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
fn bench_latency(args: &[String]) -> Result<(), String> {
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
fn bench_feed(args: &[String]) -> Result<(), String> {
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
fn kill_test(args: &[String]) -> Result<(), String> {
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
fn backup(args: &[String]) -> Result<(), String> {
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
fn bench_contention_local(args: &[String]) -> Result<(), String> {
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
