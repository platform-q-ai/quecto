//! SPIKE (board service): the board's ports over a board server, for
//! `quecto board` and the agent tool, chosen by `QUECTO_BOARD_SERVICE`
//! (`http://host:port[,http://standby:port]` or `unix:/path`). Endpoints
//! are tried in order; a write carries its own id, so a retry after a lost
//! answer (or on the standby, once promoted) is never applied twice.
use super::files::{BoardFile, snapshot_of};
use super::http::{self, Endpoint};
use super::store::encoded;
use super::wire::{
    BootstrapWire, ClearWire, ErrorWire, OutcomeWire, SnapshotWire, WireFile, WireIdentity,
    WriteWire,
};
use crate::application::project_board::dto::{BoardListing, ProjectBoardError};
use crate::application::project_board::ports::{
    BoardArtifact, BoardBootstrap, BoardClock, BoardClockError, BoardHead, BoardListings,
    BoardSnapshot, BoardStoreError, BoardWrite, ProjectBoardStore, ReferenceStatus,
    RemoteRevisions, SetAsideClear,
};
use crate::application::project_board::use_cases::QueryBoard;
use crate::domain::project_board::entities::claim::Identity;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use crate::infrastructure::time::board_clock::SystemBoardClock;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

pub const SERVICE_VARIABLE: &str = "QUECTO_BOARD_SERVICE";

pub struct ServiceBoardStore {
    endpoints: Vec<Endpoint>,
    preferred: AtomicUsize,
    writer: Identity,
    timeout: Duration,
    /// Writes the server answered lost (a race on the head).
    pub lost: AtomicU64,
    /// Calls retried on another endpoint or round.
    pub retries: AtomicU64,
}

impl ServiceBoardStore {
    /// # Errors
    /// No endpoint the text names.
    pub fn new(spec: &str, writer: Identity) -> Result<Self, String> {
        let endpoints: Vec<Endpoint> = spec
            .split(',')
            .map(|one| {
                Endpoint::parse(one.trim()).ok_or_else(|| format!("{one:?} is no board server"))
            })
            .collect::<Result<_, _>>()?;
        match endpoints.is_empty() {
            true => Err("no board server named".into()),
            false => Ok(Self {
                endpoints,
                preferred: AtomicUsize::new(0),
                writer,
                timeout: Duration::from_secs(10),
                lost: AtomicU64::new(0),
                retries: AtomicU64::new(0),
            }),
        }
    }

    /// One call, tried on each endpoint in turn (the last that answered
    /// first), three rounds: a follower's 503 or an endpoint out of reach
    /// moves on; a call that may have been taken is retried only when
    /// `idempotent`.
    fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        idempotent: bool,
    ) -> Result<(u16, Vec<u8>), BoardStoreError> {
        let mut said = vec![];
        for round in 0..3u32 {
            if round > 0 {
                std::thread::sleep(Duration::from_millis(250 * u64::from(round)));
            }
            let first = self.preferred.load(Ordering::Relaxed);
            for step in 0..self.endpoints.len() {
                let at = (first + step) % self.endpoints.len();
                if round > 0 || step > 0 {
                    self.retries.fetch_add(1, Ordering::Relaxed);
                }
                match http::call(&self.endpoints[at], method, path, body, self.timeout) {
                    Ok((503, answer)) => said.push(format!(
                        "{}: {}",
                        self.endpoints[at].text(),
                        String::from_utf8_lossy(&answer)
                    )),
                    Ok(answer) => {
                        self.preferred.store(at, Ordering::Relaxed);
                        return Ok(answer);
                    }
                    Err(error) if !error.sent || idempotent => said.push(error.why),
                    Err(error) => {
                        return Err(BoardStoreError::Unavailable(format!(
                            "the board server may or may not have taken the call: {}",
                            error.why
                        )));
                    }
                }
            }
        }
        Err(BoardStoreError::Unavailable(format!(
            "no board server answered: {}",
            said.join("; ")
        )))
    }

    fn outcome(&self, path: &str, body: &[u8]) -> Result<OutcomeWire, BoardStoreError> {
        let (_, answer) = self.call("POST", path, Some(body), true)?;
        serde_json::from_slice(&answer).map_err(|error| {
            BoardStoreError::Unavailable(format!("the board server's answer: {error}"))
        })
    }

    /// The server's clock.
    pub fn server_time(&self) -> Option<Timestamp> {
        let (status, body) = self.call("GET", "/time", None, true).ok()?;
        if status != 200 {
            return None;
        }
        let value: serde_json::Value = serde_json::from_slice(&body).ok()?;
        let text = value.get("now")?.as_str()?;
        // Seconds are what a board timestamp keeps.
        let seconds = format!("{}Z", text.get(..19)?);
        Timestamp::parse(&seconds).ok()
    }
}

impl ProjectBoardStore for ServiceBoardStore {
    fn read(&self) -> Result<BoardSnapshot, BoardStoreError> {
        let (status, body) = self.call("GET", "/snapshot", None, true)?;
        if status != 200 {
            let error: ErrorWire = serde_json::from_slice(&body).map_err(|error| {
                BoardStoreError::Unavailable(format!("status {status}: {error}"))
            })?;
            return Err(error.error());
        }
        let wire: SnapshotWire = serde_json::from_slice(&body)
            .map_err(|error| BoardStoreError::Unavailable(format!("the snapshot: {error}")))?;
        let files: Vec<BoardFile> = wire
            .files
            .into_iter()
            .map(WireFile::file)
            .collect::<Result<_, _>>()
            .map_err(BoardStoreError::Unavailable)?;
        Ok(snapshot_of(BoardHead(wire.head), &files))
    }

    fn write(
        &self,
        expected: &BoardHead,
        artifacts: &[BoardArtifact],
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        let files = encoded(artifacts)?;
        let wire = WriteWire {
            expected: expected.0.clone(),
            files: files
                .iter()
                .map(|(_, path, bytes)| {
                    WireFile::of(&BoardFile {
                        path: path.clone(),
                        mode: super::files::REGULAR.into(),
                        bytes: bytes.clone(),
                    })
                })
                .collect(),
            message: message.to_string(),
            writer: WireIdentity::from(&self.writer),
            write_id: uuid::Uuid::new_v4().to_string(),
        };
        let body = serde_json::to_vec(&wire)
            .map_err(|error| BoardStoreError::InvalidWrite(error.to_string()))?;
        let result = self.outcome("/write", &body)?.write();
        if matches!(result, Ok(BoardWrite::Lost)) {
            self.lost.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    fn clear_set_aside(
        &self,
        expected: &BoardHead,
        clear: &SetAsideClear,
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        let wire = ClearWire {
            expected: expected.0.clone(),
            path: clear.path.clone(),
            object: clear.object.clone(),
            repeated: ClearWire::repeated_of(clear),
            message: message.to_string(),
            writer: WireIdentity::from(&self.writer),
            write_id: uuid::Uuid::new_v4().to_string(),
        };
        let body = serde_json::to_vec(&wire)
            .map_err(|error| BoardStoreError::InvalidWrite(error.to_string()))?;
        self.outcome("/clear", &body)?.write()
    }

    fn bootstrap(&self) -> Result<BoardBootstrap, BoardStoreError> {
        let body = serde_json::to_vec(&BootstrapWire {
            writer: WireIdentity::from(&self.writer),
        })
        .map_err(|error| BoardStoreError::InvalidWrite(error.to_string()))?;
        self.outcome("/bootstrap", &body)?.bootstrap()
    }

    fn writer(&self) -> Identity {
        self.writer.clone()
    }
}

/// The board clock: this machine's time, referenced to the server's.
pub struct ServiceClock {
    clock: SystemBoardClock,
    store: Arc<ServiceBoardStore>,
}

impl ServiceClock {
    pub fn new(store: Arc<ServiceBoardStore>) -> Self {
        Self {
            clock: SystemBoardClock::default(),
            store,
        }
    }
}

impl BoardClock for ServiceClock {
    fn now(&self) -> Result<Timestamp, BoardClockError> {
        self.clock.now()
    }

    fn reference(&self) -> Option<Timestamp> {
        self.store.server_time()
    }

    fn reference_status(&self) -> ReferenceStatus {
        match self.reference() {
            Some(_) => ReferenceStatus::Referenced,
            None => {
                ReferenceStatus::NoReference("the board server's clock could not be read".into())
            }
        }
    }

    fn pause(&self, attempt: usize) {
        self.clock.pause(attempt);
    }

    fn elapsed(&self) -> Duration {
        self.clock.elapsed()
    }
}

/// The listings, from one snapshot each (the server's database is the
/// index).
pub struct SnapshotListings {
    query: QueryBoard,
}

impl SnapshotListings {
    pub fn new(store: Arc<dyn ProjectBoardStore>) -> Self {
        Self {
            query: QueryBoard::new(store),
        }
    }
}

fn store_error(error: ProjectBoardError) -> BoardStoreError {
    match error {
        ProjectBoardError::Store(error) => error,
        other => BoardStoreError::Unavailable(other.to_string()),
    }
}

impl BoardListings for SnapshotListings {
    fn ready(&self) -> Result<BoardListing, BoardStoreError> {
        self.query.ready().map_err(store_error)
    }

    fn takeable(&self, now: &Timestamp) -> Result<BoardListing, BoardStoreError> {
        self.query.takeable(now).map_err(store_error)
    }

    fn all(&self) -> Result<BoardListing, BoardStoreError> {
        self.query.all().map_err(store_error)
    }

    fn by_status(&self, status: TaskStatus) -> Result<BoardListing, BoardStoreError> {
        self.query.by_status(status).map_err(store_error)
    }

    fn children(&self, parent: &Slug) -> Result<BoardListing, BoardStoreError> {
        self.query.children(parent).map_err(store_error)
    }

    fn dependents(&self, task: &Slug) -> Result<BoardListing, BoardStoreError> {
        self.query.dependents(task).map_err(store_error)
    }
}

/// Spike: no git remote is consulted for a run's revisions.
pub struct NoRemoteRevisions;

impl RemoteRevisions for NoRemoteRevisions {
    fn not_on_remote(&self, _revisions: &[String]) -> Result<Vec<String>, BoardStoreError> {
        Err(BoardStoreError::Unavailable(
            "the board service spike reads no git remote".into(),
        ))
    }

    fn commit_id_hex_len(&self) -> Result<usize, BoardStoreError> {
        Ok(40)
    }
}
