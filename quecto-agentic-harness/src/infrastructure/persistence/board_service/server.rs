//! SPIKE (board service): `quecto board-server`, serving one board's
//! SQLite store over HTTP (a TCP bind address) and a Unix socket, with a
//! change feed (server-sent events, resumable from a revision), a
//! follower mode (`--follow <primary>`) that applies the primary's feed
//! into its own database and serves reads alone, and a lease on a GitHub
//! ref that a follower takes, and is promoted by, once it lapses.
use super::files::{Head, REGULAR, Read, read_file};
use super::http::{self, Endpoint, Handler, Request, Stream, respond};
use super::lease::{self, LeaseConfig, LeaseState, SharedLease};
use super::sqlite::{FileChange, RevisionMeta, RevisionRow, SqliteBoard};
use super::wire::{
    BootstrapWire, ClearWire, ErrorWire, FeedEvent, OutcomeWire, SnapshotWire, StatusWire,
    WireFile, WriteWire,
};
use crate::application::project_board::ports::{ArtifactKey, BoardStoreError};
use crate::domain::project_board::entities::claim::Identity;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Primary,
    Follower,
}

pub struct Server {
    pub board: Arc<SqliteBoard>,
    role: RwLock<Role>,
    lease: Mutex<Option<SharedLease>>,
    snapshot: Mutex<Option<(Head, Arc<Vec<u8>>)>>,
    pub stop_following: Arc<AtomicBool>,
}

fn json<T: serde::Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_default()
}

fn now_text() -> String {
    humantime::format_rfc3339_millis(std::time::SystemTime::now()).to_string()
}

impl Server {
    pub fn new(board: Arc<SqliteBoard>, role: Role) -> Arc<Self> {
        let server = Arc::new(Self {
            board,
            role: RwLock::new(role),
            lease: Mutex::new(None),
            snapshot: Mutex::new(None),
            stop_following: Arc::new(AtomicBool::new(false)),
        });
        server.set_role(role);
        server
    }

    pub fn role(&self) -> Role {
        *self.role.read().unwrap_or_else(|e| e.into_inner())
    }

    /// The role, and the write gate it brings: a follower takes no write;
    /// a primary takes them while its lease (if it has one) is held.
    pub fn set_role(self: &Arc<Self>, role: Role) {
        *self.role.write().unwrap_or_else(|e| e.into_inner()) = role;
        let me = Arc::downgrade(self);
        self.board.set_gate(Some(Arc::new(move || {
            let Some(me) = me.upgrade() else {
                return Err("the server is gone".into());
            };
            match me.role() {
                Role::Follower => Err("a read-only follower".into()),
                Role::Primary => {
                    match me.lease.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                        Some(held) => lease::writable(held),
                        None => Ok(()),
                    }
                }
            }
        })));
    }

    pub fn set_lease(&self, held: Option<SharedLease>) {
        *self.lease.lock().unwrap_or_else(|e| e.into_inner()) = held;
    }

    pub fn handler(self: &Arc<Self>) -> Handler {
        let me = Arc::clone(self);
        Arc::new(move |request, stream| me.handle(request, stream))
    }

    fn handle(&self, request: Request, mut stream: Box<dyn Stream>) {
        let started = Instant::now();
        let path = request.path.clone();
        let (status, body) = match (request.method.as_str(), path.as_str()) {
            ("GET", "/feed") => {
                let from = request
                    .query("from")
                    .and_then(|from| from.parse().ok())
                    .unwrap_or(0);
                self.feed(stream, from);
                return;
            }
            ("GET", "/status") => (200, json(&self.status())),
            ("GET", "/time") => (200, json(&serde_json::json!({ "now": now_text() }))),
            ("GET", "/snapshot") => self.snapshot(),
            ("POST", "/write") => self.write(&request.body),
            ("POST", "/clear") => self.clear(&request.body),
            ("POST", "/bootstrap") => self.bootstrap(&request.body),
            ("GET", revision) if revision.starts_with("/revision/") => {
                match revision["/revision/".len()..].parse::<u64>() {
                    Ok(n) => match self.board.revision(n) {
                        Ok((row, changes)) => (200, json(&(row, changes))),
                        Err(error) => (404, json(&ErrorWire::of(&error))),
                    },
                    Err(_) => (400, vec![]),
                }
            }
            _ => (404, vec![]),
        };
        if status == 0 {
            return;
        }
        let _ = respond(&mut stream, status, &body, started.elapsed());
    }

    fn status(&self) -> StatusWire {
        let lease = self
            .lease
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|held| {
                let held = held.lock().unwrap_or_else(|e| e.into_inner());
                format!(
                    "fenced={} valid_for_ms={} renewals={} failures={} last_renew_ms={:.0}",
                    held.fenced,
                    held.valid_until
                        .map(|until| until.saturating_duration_since(Instant::now()).as_millis())
                        .unwrap_or(0),
                    held.renewals,
                    held.failures,
                    held.last_renew_ms
                )
            });
        StatusWire {
            role: format!("{:?}", self.role()).to_lowercase(),
            head: self.board.head().ok().flatten().map(Head::text),
            lease,
            server_time: now_text(),
        }
    }

    fn snapshot(&self) -> (u16, Vec<u8>) {
        let current = match self.board.current() {
            Ok(current) => current,
            Err(error) => return (404, json(&ErrorWire::of(&error))),
        };
        let mut cached = self.snapshot.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((head, body)) = cached.as_ref() {
            if *head == current.head {
                return (200, body.to_vec());
            }
        }
        let body = Arc::new(json(&SnapshotWire {
            head: current.head.text(),
            files: current.files.iter().map(WireFile::of).collect(),
        }));
        *cached = Some((current.head, Arc::clone(&body)));
        (200, body.to_vec())
    }

    fn refused_here(&self) -> Option<(u16, Vec<u8>)> {
        match self.role() {
            Role::Primary => None,
            Role::Follower => Some((
                503,
                json(&OutcomeWire::Error(ErrorWire::of(
                    &BoardStoreError::Unavailable(
                        "a read-only follower: write to the primary".into(),
                    ),
                ))),
            )),
        }
    }

    fn write(&self, body: &[u8]) -> (u16, Vec<u8>) {
        if let Some(refused) = self.refused_here() {
            return refused;
        }
        let Ok(write) = serde_json::from_slice::<WriteWire>(body) else {
            return (400, vec![]);
        };
        let mut files: Vec<(ArtifactKey, String, Vec<u8>)> = vec![];
        for file in write.files {
            let file = match file.file() {
                Ok(file) => file,
                Err(why) => {
                    return (
                        200,
                        json(&OutcomeWire::of_write(&Err(BoardStoreError::InvalidWrite(
                            why,
                        )))),
                    );
                }
            };
            match read_file(&file.path, REGULAR, &file.bytes) {
                Read::Artifact(artifact) => files.push((artifact.key(), file.path, file.bytes)),
                Read::Aside(aside) => {
                    return (
                        200,
                        json(&OutcomeWire::of_write(&Err(BoardStoreError::InvalidWrite(
                            format!("{}: {}", aside.path, aside.reason),
                        )))),
                    );
                }
                Read::Meta => {
                    return (
                        200,
                        json(&OutcomeWire::of_write(&Err(BoardStoreError::InvalidWrite(
                            format!("{} is not an artifact's file", file.path),
                        )))),
                    );
                }
            }
        }
        let meta = RevisionMeta {
            write_id: Some(write.write_id),
            ..RevisionMeta::by(write.writer.into(), &write.message, "write")
        };
        let result = self.board.write_files(&write.expected, &files, &meta);
        // Test hook: drop the connection after every Nth commit, unanswered.
        if let (Ok(crate::application::project_board::ports::BoardWrite::Written(_)), Some(every)) = (
            &result,
            std::env::var("QUECTO_SPIKE_DROP_EVERY")
                .ok()
                .and_then(|n| n.parse::<u64>().ok())
                .filter(|n| *n > 0),
        ) {
            static COMMITS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            if COMMITS.fetch_add(1, Ordering::SeqCst) % every == every - 1 {
                return (0, vec![]);
            }
        }
        let status = match &result {
            Err(BoardStoreError::Unavailable(_)) => 503,
            _ => 200,
        };
        (status, json(&OutcomeWire::of_write(&result)))
    }

    fn clear(&self, body: &[u8]) -> (u16, Vec<u8>) {
        if let Some(refused) = self.refused_here() {
            return refused;
        }
        let Ok(clear) = serde_json::from_slice::<ClearWire>(body) else {
            return (400, vec![]);
        };
        let meta = RevisionMeta {
            write_id: Some(clear.write_id.clone()),
            ..RevisionMeta::by(clear.writer.clone().into(), &clear.message, "clear")
        };
        let result = self.board.clear(&clear.expected, &clear.clear(), &meta);
        (200, json(&OutcomeWire::of_write(&result)))
    }

    fn bootstrap(&self, body: &[u8]) -> (u16, Vec<u8>) {
        if let Some(refused) = self.refused_here() {
            return refused;
        }
        let Ok(bootstrap) = serde_json::from_slice::<BootstrapWire>(body) else {
            return (400, vec![]);
        };
        let meta = RevisionMeta::by(bootstrap.writer.into(), "board: bootstrap", "bootstrap");
        (
            200,
            json(&OutcomeWire::of_bootstrap(&self.board.bootstrap(&meta))),
        )
    }

    /// The change feed: every revision after `from`, then each as it lands,
    /// and an `epoch` event when the epoch moves; a ping every 10 s keeps
    /// it open. It reads from the database each time, so a slow subscriber
    /// is never dropped and never misses a revision.
    fn feed(&self, mut stream: Box<dyn Stream>, from: u64) {
        let head =
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\n\r\n";
        if stream
            .write_all(head.as_bytes())
            .and_then(|()| stream.flush())
            .is_err()
        {
            return;
        }
        let mut last = from;
        let mut epoch = None;
        loop {
            let generation = *self
                .board
                .latest
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let Ok(rows) = self.board.revisions_after(last, 256) else {
                return;
            };
            for row in &rows {
                let paths = self
                    .board
                    .revision(row.revision)
                    .map(|(_, changes)| changes.into_iter().map(|change| change.path).collect())
                    .unwrap_or_default();
                let event = FeedEvent {
                    epoch: row.epoch,
                    revision: row.revision,
                    kind: row.kind.clone(),
                    writer: format!("{} <{}>", row.writer_name, row.writer_email),
                    message: row.message.clone(),
                    server_time: row.server_time.clone(),
                    paths,
                };
                let line = format!(
                    "event: revision\ndata: {}\n\n",
                    serde_json::to_string(&event).unwrap_or_default()
                );
                if stream
                    .write_all(line.as_bytes())
                    .and_then(|()| stream.flush())
                    .is_err()
                {
                    return;
                }
                last = row.revision;
            }
            if let Ok(Some(now)) = self.board.head() {
                if epoch != Some(now.epoch) {
                    let line = format!(
                        "event: epoch\ndata: {{\"epoch\":{},\"revision\":{}}}\n\n",
                        now.epoch, now.revision
                    );
                    if stream
                        .write_all(line.as_bytes())
                        .and_then(|()| stream.flush())
                        .is_err()
                    {
                        return;
                    }
                    epoch = Some(now.epoch);
                }
            }
            if rows.len() == 256 {
                continue;
            }
            let (counter, waiters) = &*self.board.latest;
            let guard = counter.lock().unwrap_or_else(|e| e.into_inner());
            let (guard, timeout) = waiters
                .wait_timeout_while(guard, Duration::from_secs(10), |now| *now == generation)
                .unwrap_or_else(|e| e.into_inner());
            drop(guard);
            if timeout.timed_out()
                && stream
                    .write_all(b": ping\n\n")
                    .and_then(|()| stream.flush())
                    .is_err()
            {
                return;
            }
        }
    }
}

/// The metadata a follower records a primary's revision with, as it was.
fn meta_of(row: &RevisionRow) -> RevisionMeta {
    RevisionMeta {
        write_id: row.write_id.clone(),
        writer: Identity {
            name: row.writer_name.clone(),
            email: row.writer_email.clone(),
        },
        principal: row.principal.clone(),
        message: row.message.clone(),
        kind: row.kind.clone(),
        git_commit: row.git_commit.clone(),
        git_meta: row.git_meta.clone(),
        server_time: Some(row.server_time.clone()),
    }
}

/// Applies revision `n` from `primary` into `board`.
fn apply(board: &SqliteBoard, primary: &Endpoint, n: u64) -> Result<(), String> {
    let (status, body) = http::call(
        primary,
        "GET",
        &format!("/revision/{n}"),
        None,
        Duration::from_secs(10),
    )
    .map_err(|error| error.why)?;
    if status != 200 {
        return Err(format!("revision {n}: {status}"));
    }
    let (row, changes): (RevisionRow, Vec<FileChange>) =
        serde_json::from_slice(&body).map_err(|error| error.to_string())?;
    board
        .append_raw(
            Head {
                epoch: row.epoch,
                revision: row.revision,
            },
            &changes,
            &meta_of(&row),
        )
        .map_err(|error| format!("{error:?}"))
}

/// Follows `primary`'s feed into `server`'s board until told to stop:
/// each revision fetched and applied in order, resuming from its own head
/// after any break.
pub fn follow(server: Arc<Server>, primary: Endpoint) {
    std::thread::spawn(move || {
        let stop = Arc::clone(&server.stop_following);
        while !stop.load(Ordering::SeqCst) {
            let from = server
                .board
                .head()
                .ok()
                .flatten()
                .map_or(0, |head| head.revision);
            let result = http::events(
                &primary,
                &format!("/feed?from={from}"),
                Duration::from_secs(30),
                |event, data| {
                    if stop.load(Ordering::SeqCst) {
                        return false;
                    }
                    if event != "revision" {
                        return true;
                    }
                    let Ok(event) = serde_json::from_str::<FeedEvent>(data) else {
                        return false;
                    };
                    let local = server
                        .board
                        .head()
                        .ok()
                        .flatten()
                        .map_or(0, |head| head.revision);
                    for n in local + 1..=event.revision {
                        if let Err(why) = apply(&server.board, &primary, n) {
                            eprintln!("follow: {why}");
                            return false;
                        }
                    }
                    true
                },
            );
            if let Err(error) = result {
                eprintln!("follow: feed from {} broke: {error}", primary.text());
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    });
}

/// The command line: `quecto board-server --db <file> [--listen <addr>]
/// [--socket <path>] [--follow <url>] [--lease-url <git url> --lease-ref
/// <ref> --holder <name> [--lease-ttl <s>] [--lease-every <s>]
/// [--lease-git <dir>]]`, or one of the tools: `import`, `export`,
/// `bench-*` (see `tools`).
pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some(
            "import" | "export" | "verify-export" | "bench-feed" | "bench-latency" | "kill-test"
            | "backup",
        ) => super::tools::run(args),
        _ => serve(args),
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|at| args.get(at + 1))
        .cloned()
}

fn serve(args: &[String]) -> i32 {
    let Some(db) = flag(args, "--db") else {
        eprintln!(
            "usage: quecto board-server --db <file> [--listen <addr>] [--socket <path>] [--follow <url>] [--lease-url ...]"
        );
        return 2;
    };
    let started = Instant::now();
    let board = match SqliteBoard::open(&PathBuf::from(&db)) {
        Ok(board) => Arc::new(board),
        Err(error) => {
            eprintln!("board-server: {error:?}");
            return 1;
        }
    };
    let follow_url = flag(args, "--follow").and_then(|url| Endpoint::parse(&url));
    let role = match follow_url {
        Some(_) => Role::Follower,
        None => Role::Primary,
    };
    let server = Server::new(Arc::clone(&board), role);
    let lease_config = flag(args, "--lease-url").map(|url| LeaseConfig {
        url,
        reference: flag(args, "--lease-ref")
            .unwrap_or_else(|| "refs/quecto/board-lease/spike".into()),
        holder: flag(args, "--holder").unwrap_or_else(|| "board-server".into()),
        ttl: Duration::from_secs_f64(
            flag(args, "--lease-ttl")
                .and_then(|s| s.parse().ok())
                .unwrap_or(15.0),
        ),
        every: Duration::from_secs_f64(
            flag(args, "--lease-every")
                .and_then(|s| s.parse().ok())
                .unwrap_or(3.0),
        ),
        git_dir: PathBuf::from(
            flag(args, "--lease-git").unwrap_or_else(|| format!("{db}.lease-git")),
        ),
    });
    if let Some(config) = &lease_config {
        if let Err(why) = lease::prepare(config) {
            eprintln!("board-server: lease: {why}");
            return 1;
        }
    }
    let mut listening = vec![];
    if let Some(address) = flag(args, "--listen") {
        match std::net::TcpListener::bind(&address) {
            Ok(listener) => {
                let handler = server.handler();
                std::thread::spawn(move || http::serve_tcp(listener, handler));
                listening.push(format!("http://{address}"));
            }
            Err(error) => {
                eprintln!("board-server: bind {address}: {error}");
                return 1;
            }
        }
    }
    if let Some(socket) = flag(args, "--socket") {
        let _ = std::fs::remove_file(&socket);
        match std::os::unix::net::UnixListener::bind(&socket) {
            Ok(listener) => {
                let handler = server.handler();
                std::thread::spawn(move || http::serve_unix(listener, handler));
                listening.push(format!("unix:{socket}"));
            }
            Err(error) => {
                eprintln!("board-server: bind {socket}: {error}");
                return 1;
            }
        }
    }
    eprintln!(
        "board-server: {role:?} on {} (db {db}, head {:?}), up in {:.1} ms",
        listening.join(", "),
        board.head().ok().flatten().map(Head::text),
        started.elapsed().as_secs_f64() * 1000.0
    );
    match (follow_url, lease_config) {
        (Some(primary), Some(config)) => {
            follow(Arc::clone(&server), primary);
            standby(&server, config);
        }
        (Some(primary), None) => follow(Arc::clone(&server), primary),
        (None, Some(config)) => {
            // No write until the lease is held.
            server.set_lease(Some(Arc::new(Mutex::new(LeaseState::default()))));
            if !take_lease(&server, &config, false) {
                return 1;
            }
        }
        (None, None) => {}
    }
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

/// Takes the lease (waiting for it to lapse unless it is absent) and holds
/// it: on success the server is primary at a new epoch when `promote`.
fn take_lease(server: &Arc<Server>, config: &LeaseConfig, promote: bool) -> bool {
    let never = AtomicBool::new(false);
    loop {
        let Some(seen) = lease::await_lapse(config, &never) else {
            return false;
        };
        let epoch_now = server
            .board
            .head()
            .ok()
            .flatten()
            .map_or(1, |head| head.epoch);
        let epoch = match promote {
            true => epoch_now + 1,
            false => epoch_now,
        };
        let taken_at = Instant::now();
        match lease::swap(config, seen.as_deref(), epoch) {
            Ok(Some(oid)) => {
                let state = Arc::new(Mutex::new(LeaseState {
                    valid_until: Some(taken_at + config.ttl - config.ttl / 4),
                    last_oid: Some(oid),
                    ..LeaseState::default()
                }));
                server.set_lease(Some(Arc::clone(&state)));
                if promote {
                    server.stop_following.store(true, Ordering::SeqCst);
                    match server
                        .board
                        .bump_epoch("promoted: the primary's lease lapsed", epoch)
                    {
                        Ok(head) => eprintln!(
                            "board-server: PROMOTED to primary at {} ({} ms after the lease lapsed)",
                            head.text(),
                            taken_at.elapsed().as_millis()
                        ),
                        Err(error) => eprintln!("board-server: promotion: {error:?}"),
                    }
                    server.set_role(Role::Primary);
                }
                eprintln!("board-server: holds the lease as {}", config.holder);
                lease::renew_forever(
                    config.clone(),
                    state,
                    epoch,
                    Arc::new(AtomicBool::new(false)),
                );
                return true;
            }
            Ok(None) => eprintln!("board-server: lost the race for the lease; watching again"),
            Err(why) => eprintln!("board-server: lease take failed: {why}"),
        }
    }
}

/// A standby: watches the lease while following, and takes it, promoting
/// itself, once it lapses.
fn standby(server: &Arc<Server>, config: LeaseConfig) {
    let server = Arc::clone(server);
    std::thread::spawn(move || {
        take_lease(&server, &config, true);
    });
}
