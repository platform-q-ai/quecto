//! An outside reading of the time for the project board's clock (#2491):
//! the `Date` header the board remote's HTTPS host answers.
//!
//! **Where it is read.** From the remote's URL:
//! - an `https://` remote is asked at its own host and port;
//! - an SSH remote (`ssh://`, `git+ssh://` or `user@host:path`) gives no
//!   time of its own, so its host is mapped to HTTPS only when it is a
//!   known forge ([`KNOWN_FORGES`], `ssh.github.com` included);
//! - any other remote (another SSH host, `http://`, `git://`, a path) has
//!   no reference. Board changes are then not bounded against an outside
//!   time; [`ReferencedBoardClock::reference_status`] says so for a front
//!   end to show, and a warning is logged once.
//!
//! **How it is read.** A `HEAD` with `Cache-Control: no-cache`, whose
//! `Date` plus any `Age` is the reference, stamped when the answer came.
//! One client is kept for the clock's life, built on the first reading's
//! own thread. Each reading runs on its own thread and is waited for at
//! most [`READ_TIMEOUT`], DNS included; a reading that has not answered by
//! then is abandoned. The client honours the usual `HTTPS_PROXY` and
//! `NO_PROXY` variables, and trusts the public web roots only, so a forge
//! behind a private CA gives no reference. A reading more than
//! [`MAX_PLAUSIBLE_OFFSET`] from this machine's time is not believed.
//!
//! **How long it stands.** A reading is kept with the monotonic and wall
//! times it was taken at; the reference advances with the monotonic clock.
//! It is read again after [`REFRESH_EVERY`], or as soon as the two clocks
//! disagree by more than [`SUSPEND_TOLERANCE`] (a suspend stops the
//! monotonic clock, which would make the reference lag the real time). A
//! failed reading stands [`RETRY_FAILED_AFTER`], so an unreachable host
//! costs one timeout that often. One reading runs at a time: a caller that
//! finds one in flight is served the last reading if it merely aged out,
//! and otherwise waits for the one in flight, at most the read timeout.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, mpsc};
use std::time::{Duration, Instant, SystemTime};

use reqwest::Url;

use crate::application::project_board::ports::{BoardClock, BoardClockError, ReferenceStatus};
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::redaction::redact_url_userinfo;

use super::board_clock::{SystemBoardClock, timestamp_at};

/// SSH hosts of forges that answer HTTPS, and the HTTPS host each answers
/// on.
pub const KNOWN_FORGES: &[(&str, &str)] = &[
    ("github.com", "github.com"),
    ("ssh.github.com", "github.com"),
    ("gitlab.com", "gitlab.com"),
    ("altssh.gitlab.com", "gitlab.com"),
];
/// The longest a reading of the reference may take, DNS included.
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a reading stands.
pub const REFRESH_EVERY: Duration = Duration::from_secs(10 * 60);
/// How long a failure to read stands before it is tried again.
pub const RETRY_FAILED_AFTER: Duration = Duration::from_secs(45);
/// How far the wall and monotonic clocks may drift apart over a reading's
/// life before it is read again.
pub const SUSPEND_TOLERANCE: Duration = Duration::from_secs(5);
/// How far an outside reading may be from this machine's time and still
/// be believed.
pub const MAX_PLAUSIBLE_OFFSET: Duration = Duration::from_secs(24 * 60 * 60);

/// Where a remote's reference time is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceSource {
    /// The URL whose `Date` header is the reference.
    Https(String),
    /// No reference, and why (with no credentials in it).
    Unavailable(String),
}

/// Where the reference time of the remote at `remote_url` is read.
pub fn reference_source(remote_url: &str) -> ReferenceSource {
    let shown = redact_url_userinfo(remote_url.trim());
    let none = |why: &str| ReferenceSource::Unavailable(format!("{why} ({shown})"));
    let Some(url) = as_url(remote_url.trim()) else {
        return none("a remote that is neither an https nor an ssh URL gives no time");
    };
    let Some(host) = url
        .host_str()
        .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
    else {
        return none("a remote URL without a host gives no time");
    };
    let source = match url.scheme() {
        "https" => https_url(&host, url.port()),
        "ssh" | "git+ssh" => match KNOWN_FORGES.iter().find(|(ssh, _)| *ssh == host) {
            Some((_, https)) => https_url(https, None),
            None => return none("an ssh remote that is not a known forge gives no time"),
        },
        _ => return none("a remote over neither https nor ssh gives no time"),
    };
    match source {
        Some(url) => {
            assert!(
                !url.contains('@') && url.ends_with('/'),
                "a bare host URL: {url}"
            );
            ReferenceSource::Https(url)
        }
        None => none("a remote whose host cannot be read gives no time"),
    }
}

/// `remote` as a URL; git's scp-like `[user@]host:path` is read as the
/// `ssh://` URL it names.
fn as_url(remote: &str) -> Option<Url> {
    let scheme_like = remote.split_once("://").is_some_and(|(scheme, _)| {
        scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+')
    });
    if scheme_like {
        return Url::parse(remote).ok();
    }
    let (host, path) = remote.split_once(':')?;
    let host_shaped = !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'@'));
    match host_shaped && !path.starts_with('/') {
        true => Url::parse(&format!("ssh://{host}/{path}")).ok(),
        false => None,
    }
}

/// `https://<host>[:<port>]/`, for a host of letters, digits, dots and
/// dashes only.
fn https_url(host: &str, port: Option<u16>) -> Option<String> {
    let shaped = !host.is_empty()
        && host.contains('.')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'));
    shaped.then(|| match port {
        Some(port) => format!("https://{host}:{port}/"),
        None => format!("https://{host}/"),
    })
}

/// One reading of an outside time.
pub trait TimeReading: Send + Sync {
    /// The time the source reports now.
    ///
    /// # Errors
    /// Why it could not be read.
    fn read(&self) -> Result<SystemTime, String>;
}

/// The `Date` (plus `Age`) of a `HEAD` request to one URL. Its client is
/// built on the first reading's own thread (a blocking client runs a
/// runtime of its own, which a caller's thread need not host) and kept.
pub struct HttpDate {
    url: String,
    proxied: bool,
    client: OnceLock<Result<reqwest::blocking::Client, String>>,
}

impl HttpDate {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            proxied: true,
            client: OnceLock::new(),
        }
    }

    /// The same, never through a proxy: for a source on this machine.
    pub fn without_proxy(url: impl Into<String>) -> Self {
        Self {
            proxied: false,
            ..Self::new(url)
        }
    }

    fn client(&self) -> Result<&reqwest::blocking::Client, String> {
        let built = self.client.get_or_init(|| {
            let builder = reqwest::blocking::Client::builder();
            let builder = match self.proxied {
                true => builder,
                false => builder.no_proxy(),
            };
            builder
                .timeout(READ_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|error| error.to_string())
        });
        built.as_ref().map_err(Clone::clone)
    }
}

impl TimeReading for HttpDate {
    fn read(&self) -> Result<SystemTime, String> {
        let client = self.client()?;
        let response = client
            .head(&self.url)
            .header(reqwest::header::CACHE_CONTROL, "no-cache")
            .send()
            .map_err(|error| format!("{}: {error}", self.url))?;
        let headers = response.headers();
        let date = headers
            .get(reqwest::header::DATE)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| format!("{} sent no readable Date header", self.url))?;
        let date = httpdate::parse_http_date(date)
            .map_err(|_| format!("{} sent Date {date:?}", self.url))?;
        let age = headers
            .get(reqwest::header::AGE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map_or(Duration::ZERO, Duration::from_secs);
        date.checked_add(age)
            .ok_or_else(|| format!("{} sent a Date past what can be read", self.url))
    }
}

/// This machine's two clocks: the monotonic one a reading advances with,
/// and the wall one a suspend is told by.
pub trait LocalClocks: Send + Sync {
    fn instant(&self) -> Instant;
    fn wall(&self) -> SystemTime;
}

/// The real clocks.
pub struct SystemClocks;

impl LocalClocks for SystemClocks {
    fn instant(&self) -> Instant {
        Instant::now()
    }

    fn wall(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// A reading: when it was taken by both local clocks, and the outside time
/// or why there is none.
#[derive(Clone)]
struct Reading {
    taken: Instant,
    taken_wall: SystemTime,
    outside: Result<SystemTime, String>,
}

#[derive(Default)]
struct State {
    last: Option<Reading>,
    in_flight: bool,
}

/// Clears the in-flight mark when the reading ends, however it ends, and
/// wakes the callers waiting for it.
struct InFlight<'a>(&'a ReferencedBoardClock);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.in_flight = false;
        drop(state);
        self.0.read_done.notify_all();
    }
}

/// The board clock with an outside reference: this machine's time, checked
/// against a reading of an outside source kept fresh per process.
pub struct ReferencedBoardClock {
    clock: SystemBoardClock,
    source: Result<Arc<dyn TimeReading>, String>,
    local: Box<dyn LocalClocks>,
    every: Duration,
    read_timeout: Duration,
    state: Mutex<State>,
    read_done: Condvar,
    warned: AtomicBool,
    warn: Box<dyn Fn(&str) + Send + Sync>,
}

impl ReferencedBoardClock {
    /// The clock for the board on the remote at `remote_url`.
    pub fn for_remote(remote_url: &str) -> Self {
        let source: Result<Arc<dyn TimeReading>, String> = match reference_source(remote_url) {
            ReferenceSource::Https(url) => Ok(Arc::new(HttpDate::new(url))),
            ReferenceSource::Unavailable(why) => Err(why),
        };
        Self::new(source, REFRESH_EVERY, |why| {
            tracing::warn!(
                target: "quecto::project_board",
                "board clock has no outside reference: {why}"
            );
        })
    }

    /// A clock reading `source` (or none, and why) every `every`, telling
    /// `warn` once why it has no reference.
    pub fn new(
        source: Result<Arc<dyn TimeReading>, String>,
        every: Duration,
        warn: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self {
        Self {
            clock: SystemBoardClock::default(),
            source,
            local: Box::new(SystemClocks),
            every,
            read_timeout: READ_TIMEOUT,
            state: Mutex::new(State::default()),
            read_done: Condvar::new(),
            warned: AtomicBool::new(false),
            warn: Box::new(warn),
        }
    }

    /// The same, waiting at most `timeout` for a reading.
    pub fn with_read_timeout(mut self, timeout: Duration) -> Self {
        self.read_timeout = timeout;
        self
    }

    /// The same, told the time by `local`.
    pub fn with_local_clocks(mut self, local: impl LocalClocks + 'static) -> Self {
        self.local = Box::new(local);
        self
    }

    fn warn_once(&self, why: &str) {
        if !self.warned.swap(true, Ordering::Relaxed) {
            (self.warn)(why);
        }
    }

    /// The monotonic and wall times since `reading` was taken, and whether
    /// the two clocks still agree (no suspend between).
    fn age(&self, reading: &Reading) -> (Duration, bool) {
        let monotonic = self
            .local
            .instant()
            .saturating_duration_since(reading.taken);
        let wall = self
            .local
            .wall()
            .duration_since(reading.taken_wall)
            .unwrap_or(Duration::MAX);
        (monotonic, wall.abs_diff(monotonic) <= SUSPEND_TOLERANCE)
    }

    /// The reading still standing: younger than its period (a failure's
    /// is shorter), and taken with the two local clocks still agreeing.
    fn standing(&self, reading: &Reading) -> bool {
        let (monotonic, agreeing) = self.age(reading);
        let period = match reading.outside {
            Ok(_) => self.every,
            Err(_) => self.every.min(RETRY_FAILED_AFTER),
        };
        monotonic < period && agreeing
    }

    /// The standing reading, or a fresh one taken now (outside the lock).
    /// While another caller's reading is in flight, a reading that merely
    /// aged out (no suspend since) still serves; otherwise the caller waits
    /// for the reading in flight, at most the read timeout.
    fn current(&self) -> Option<Reading> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            match (&state.last, state.in_flight) {
                (Some(last), _) if self.standing(last) => return Some(last.clone()),
                (Some(last), true) if last.outside.is_ok() && self.age(last).1 => {
                    return Some(last.clone());
                }
                (_, true) => {
                    let (waited, timeout) = self
                        .read_done
                        .wait_timeout_while(state, self.read_timeout, |state| state.in_flight)
                        .unwrap_or_else(PoisonError::into_inner);
                    state = waited;
                    if timeout.timed_out() {
                        return None;
                    }
                }
                (_, false) => {
                    state.in_flight = true;
                    break;
                }
            }
        }
        drop(state);
        let in_flight = InFlight(self);
        let reading = self.read_now();
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last = Some(reading.clone());
        drop(in_flight);
        if let Err(why) = &reading.outside {
            self.warn_once(why);
        }
        Some(reading)
    }

    /// A reading taken now, waited for at most the read timeout
    /// ([`READ_TIMEOUT`] unless set), and stamped when its answer came.
    fn read_now(&self) -> Reading {
        let outside = match &self.source {
            Err(why) => Err(why.clone()),
            Ok(source) => {
                let source = Arc::clone(source);
                let (sender, receiver) = mpsc::channel();
                let spawned = std::thread::Builder::new()
                    .name("board-clock-reading".into())
                    .spawn(move || {
                        let _ = sender.send(source.read());
                    });
                match spawned {
                    Err(error) => Err(format!("the reading could not start: {error}")),
                    Ok(_) => match receiver.recv_timeout(self.read_timeout) {
                        Ok(read) => read,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            Err(format!("no answer within {:?}", self.read_timeout))
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            Err("the reading failed before it answered".into())
                        }
                    },
                }
            }
        };
        let (taken, taken_wall) = (self.local.instant(), self.local.wall());
        let outside = outside.and_then(|time| {
            let offset = time
                .duration_since(taken_wall)
                .unwrap_or_else(|behind| behind.duration());
            match offset <= MAX_PLAUSIBLE_OFFSET {
                true => Ok(time),
                false => Err(format!(
                    "an outside time {} s from this machine's is not believed",
                    offset.as_secs()
                )),
            }
        });
        Reading {
            taken,
            taken_wall,
            outside,
        }
    }
}

impl BoardClock for ReferencedBoardClock {
    fn now(&self) -> Result<Timestamp, BoardClockError> {
        self.clock.now()
    }

    /// The standing reading advanced by the monotonic time since it was
    /// taken.
    fn reference(&self) -> Option<Timestamp> {
        let reading = self.current()?;
        let since = self
            .local
            .instant()
            .saturating_duration_since(reading.taken);
        let now = reading.outside.ok()?.checked_add(since)?;
        timestamp_at(now).ok()
    }

    fn reference_status(&self) -> ReferenceStatus {
        match self.current().map(|reading| reading.outside) {
            Some(Ok(_)) => ReferenceStatus::Referenced,
            Some(Err(why)) => ReferenceStatus::NoReference(why),
            None => ReferenceStatus::NoReference("a reading is in flight".into()),
        }
    }

    fn pause(&self, attempt: usize) {
        self.clock.pause(attempt);
    }

    fn elapsed(&self) -> Duration {
        self.clock.elapsed()
    }
}

#[cfg(test)]
#[path = "remote_time_tests.rs"]
mod tests;
