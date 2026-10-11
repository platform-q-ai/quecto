//! An outside reading of the time for the project board's clock (#2491):
//! the `Date` header the board remote's HTTPS host answers.
//!
//! The reference comes from the remote's URL:
//! - An `https://` remote is asked at its own host.
//! - An SSH remote (`ssh://` or `user@host:path`) gives no time of its own.
//!   Its host is mapped to the same host over HTTPS only when it is a
//!   known forge ([`KNOWN_FORGES`]), which answers HTTPS on the host it
//!   serves SSH on.
//! - Any other remote (another SSH host, `http://`, `git://`, a path) runs
//!   with no reference. A warning is logged once, and board changes are
//!   not bounded against an outside time.
//!
//! A reading is kept with the local monotonic instant it was taken at, so
//! the reference advances with this process's steady clock. It is read
//! again every [`REFRESH_EVERY`], and a failed read is retried no sooner,
//! so an unreachable host costs one timeout per period, not one per
//! change.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use crate::application::project_board::ports::{BoardClock, BoardClockError};
use crate::domain::project_board::value_objects::timestamp::Timestamp;

use super::board_clock::{SystemBoardClock, timestamp_at};

/// SSH hosts that answer HTTPS on the same name.
pub const KNOWN_FORGES: &[&str] = &["github.com", "gitlab.com"];
/// The longest a reading of the reference may take.
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a reading (or a failure to read) stands before the reference
/// is read again.
pub const REFRESH_EVERY: Duration = Duration::from_secs(10 * 60);

/// Where a remote's reference time is read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceSource {
    /// The URL whose `Date` header is the reference.
    Https(String),
    /// No reference, and why.
    Unavailable(String),
}

/// Where the reference time of the remote at `remote_url` is read.
pub fn reference_source(remote_url: &str) -> ReferenceSource {
    let url = remote_url.trim();
    let none = |why: &str| ReferenceSource::Unavailable(format!("{why} ({url})"));
    if let Some(rest) = url.strip_prefix("https://") {
        return match authority(rest) {
            Some((host, port)) => ReferenceSource::Https(format!("https://{host}{port}/")),
            None => none("an https remote whose host cannot be read"),
        };
    }
    let ssh_host = match url.strip_prefix("ssh://") {
        Some(rest) => authority(rest).map(|(host, _)| host),
        // `[user@]host:path`, git's scp-like form: no scheme, a colon
        // before any slash.
        None if !url.contains("://") => url
            .split_once(':')
            .filter(|(host, _)| !host.contains('/'))
            .and_then(|(host, _)| authority(host))
            .map(|(host, _)| host),
        None => return none("a remote over neither https nor ssh gives no time"),
    };
    match ssh_host {
        Some(host) if KNOWN_FORGES.contains(&host.as_str()) => {
            ReferenceSource::Https(format!("https://{host}/"))
        }
        Some(_) => none("an ssh remote that is not a known forge gives no time"),
        None => none("a remote that is not a URL gives no time"),
    }
}

/// The lowercase host and the `:port` (or nothing) of `rest`, the part of
/// a URL after its scheme, without any user.
fn authority(rest: &str) -> Option<(String, String)> {
    let authority = rest.split(['/', '?', '#']).next()?;
    let hostport = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let (host, port) = match hostport.split_once(':') {
        Some((host, port)) => (host, port),
        None => (hostport, ""),
    };
    let host_ok = !host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'));
    let port_ok = port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit());
    match (host_ok, port_ok, port.is_empty()) {
        (true, true, true) => Some((host.to_ascii_lowercase(), String::new())),
        (true, true, false) => Some((host.to_ascii_lowercase(), format!(":{port}"))),
        _ => None,
    }
}

/// One reading of an outside time.
pub trait TimeReading: Send + Sync {
    /// The time the source reports now.
    ///
    /// # Errors
    /// Why it could not be read.
    fn read(&self) -> Result<SystemTime, String>;
}

/// The `Date` header of a `HEAD` request to `url`, within [`READ_TIMEOUT`].
pub struct HttpDate {
    url: String,
    timeout: Duration,
}

impl HttpDate {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            timeout: READ_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl TimeReading for HttpDate {
    fn read(&self) -> Result<SystemTime, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(self.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| error.to_string())?;
        let response = client
            .head(&self.url)
            .send()
            .map_err(|error| format!("{}: {error}", self.url))?;
        let date = response
            .headers()
            .get(reqwest::header::DATE)
            .ok_or_else(|| format!("{} sent no Date header", self.url))?;
        let text = date
            .to_str()
            .map_err(|_| format!("{} sent an unreadable Date header", self.url))?;
        httpdate::parse_http_date(text).map_err(|_| format!("{} sent Date {text:?}", self.url))
    }
}

/// A reading and the local instant it was taken at; `None` when it failed.
type Reading = (Instant, Option<SystemTime>);

/// The board clock with an outside reference: this machine's time, checked
/// against a reading of an outside source kept fresh per process.
pub struct ReferencedBoardClock {
    clock: SystemBoardClock,
    source: Result<Box<dyn TimeReading>, String>,
    every: Duration,
    last: Mutex<Option<Reading>>,
    warned: AtomicBool,
    warn: Box<dyn Fn(&str) + Send + Sync>,
}

impl ReferencedBoardClock {
    /// The clock for the board on the remote at `remote_url`.
    pub fn for_remote(remote_url: &str) -> Self {
        let source: Result<Box<dyn TimeReading>, String> = match reference_source(remote_url) {
            ReferenceSource::Https(url) => Ok(Box::new(HttpDate::new(url))),
            ReferenceSource::Unavailable(why) => Err(why),
        };
        Self::new(source, REFRESH_EVERY, |why| {
            tracing::warn!(target: "quecto::project_board", "board clock has no outside reference: {why}");
        })
    }

    /// A clock reading `source` (or none, and why) every `every`, telling
    /// `warn` once why it has no reference.
    pub fn new(
        source: Result<Box<dyn TimeReading>, String>,
        every: Duration,
        warn: impl Fn(&str) + Send + Sync + 'static,
    ) -> Self {
        Self {
            clock: SystemBoardClock::default(),
            source,
            every,
            last: Mutex::new(None),
            warned: AtomicBool::new(false),
            warn: Box::new(warn),
        }
    }

    fn warn_once(&self, why: &str) {
        if !self.warned.swap(true, Ordering::Relaxed) {
            (self.warn)(why);
        }
    }

    /// A fresh reading, taken now.
    fn reading(&self) -> Reading {
        let taken = Instant::now();
        match &self.source {
            Ok(source) => match source.read() {
                Ok(time) => (taken, Some(time)),
                Err(why) => {
                    self.warn_once(&why);
                    (taken, None)
                }
            },
            Err(why) => {
                self.warn_once(why);
                (taken, None)
            }
        }
    }
}

impl BoardClock for ReferencedBoardClock {
    fn now(&self) -> Result<Timestamp, BoardClockError> {
        self.clock.now()
    }

    /// The last reading advanced by the local monotonic time since it was
    /// taken; read again when it is older than the refresh period.
    fn reference(&self) -> Option<Timestamp> {
        let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        let (taken, time) = match *last {
            Some((taken, time)) if taken.elapsed() < self.every => (taken, time),
            _ => {
                let reading = self.reading();
                *last = Some(reading);
                reading
            }
        };
        let now = time?.checked_add(taken.elapsed())?;
        timestamp_at(now).ok()
    }

    fn pause(&self, attempt: usize) {
        self.clock.pause(attempt);
    }
}

#[cfg(test)]
#[path = "remote_time_tests.rs"]
mod tests;
