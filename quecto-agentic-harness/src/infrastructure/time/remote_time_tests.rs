use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::AtomicUsize;

#[test]
fn https_remotes_are_asked_at_their_own_host_and_ssh_only_at_a_known_forge() {
    let https = |url: &str| ReferenceSource::Https(url.into());
    for (remote, source) in [
        (
            "https://github.com/platform-q-ai/quecto.git",
            https("https://github.com/"),
        ),
        ("HTTPS://GitHub.com/org/repo", https("https://github.com/")),
        (
            "https://user:token@Git.Example.org:8443/a/b.git",
            https("https://git.example.org:8443/"),
        ),
        (
            "https://github.com:443/org/repo",
            https("https://github.com/"),
        ),
        ("https://github.com./org/repo", https("https://github.com/")),
        (
            "git@github.com:platform-q-ai/quecto.git",
            https("https://github.com/"),
        ),
        (
            "ssh://git@GitLab.com:22/group/repo.git",
            https("https://gitlab.com/"),
        ),
        (
            "git+ssh://git@github.com/org/repo",
            https("https://github.com/"),
        ),
        (
            "ssh://git@ssh.github.com:443/org/repo",
            https("https://github.com/"),
        ),
        (
            "ssh://git@altssh.gitlab.com:443/g/r",
            https("https://gitlab.com/"),
        ),
    ] {
        assert_eq!(reference_source(remote), source, "{remote}");
    }
    for remote in [
        "git@git.example.org:repo.git",
        "ssh://git@bitbucket.org/org/repo.git",
        "http://github.com/org/repo.git",
        "git://github.com/org/repo.git",
        "/srv/git/repo.git",
        "file:///srv/git/repo.git",
        "https://ex ample.com/repo",
        "https://github.com:70000/org/repo",
        "https:///repo",
        "https://localhost/repo",
        "",
    ] {
        assert!(
            matches!(reference_source(remote), ReferenceSource::Unavailable(_)),
            "{remote}"
        );
    }
    let ReferenceSource::Unavailable(why) =
        reference_source("ssh://alice:hunter2@git.example.org/repo.git")
    else {
        panic!("no reference")
    };
    assert!(!why.contains("hunter2"), "{why}");
}

/// A one-request HTTP server answering `date` and `age`; its URL, and the
/// request it received.
fn serves_date(date: SystemTime, age: Option<u64>) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let (sender, received) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 2048];
        let read = stream.read(&mut request).unwrap_or(0);
        let _ = sender.send(String::from_utf8_lossy(&request[..read]).to_lowercase());
        let age = age.map_or(String::new(), |age| format!("Age: {age}\r\n"));
        let answer = format!(
            "HTTP/1.1 200 OK\r\nDate: {}\r\n{age}Content-Length: 0\r\nConnection: close\r\n\r\n",
            httpdate::fmt_http_date(date)
        );
        stream.write_all(answer.as_bytes()).unwrap();
    });
    (url, received)
}

fn closed_port() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    drop(listener);
    url
}

#[test]
fn the_date_header_plus_its_age_is_read_uncached_and_an_unreachable_host_is_an_error() {
    let hour_ago = SystemTime::now() - Duration::from_secs(3600);
    let (url, request) = serves_date(hour_ago, Some(120));
    let read = HttpDate::without_proxy(url).read().unwrap();
    let expected = hour_ago + Duration::from_secs(120);
    let off = expected
        .duration_since(read)
        .unwrap_or_else(|ahead| ahead.duration());
    assert!(off < Duration::from_secs(1), "{off:?}");
    let request = request.recv().unwrap();
    assert!(request.starts_with("head "), "{request}");
    assert!(request.contains("cache-control: no-cache"), "{request}");
    assert!(HttpDate::without_proxy(closed_port()).read().is_err());
}

/// The local clocks as a test sets them.
#[derive(Clone)]
struct SetClocks(Arc<Mutex<(Instant, SystemTime)>>);

impl SetClocks {
    fn new() -> Self {
        Self(Arc::new(Mutex::new((Instant::now(), SystemTime::now()))))
    }

    /// Both clocks run on `by`, or only the wall clock (a suspend).
    fn advance(&self, by: Duration, monotonic: bool) {
        let mut clocks = self.0.lock().unwrap();
        if monotonic {
            clocks.0 += by;
        }
        clocks.1 += by;
    }
}

impl LocalClocks for SetClocks {
    fn instant(&self) -> Instant {
        self.0.lock().unwrap().0
    }

    fn wall(&self) -> SystemTime {
        self.0.lock().unwrap().1
    }
}

/// A source reporting a fixed `time` (or failing), counting its reads.
struct Fixed {
    time: Result<SystemTime, String>,
    reads: Arc<AtomicUsize>,
}

impl TimeReading for Fixed {
    fn read(&self) -> Result<SystemTime, String> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.time.clone()
    }
}

fn clock_of(
    time: Result<SystemTime, String>,
    every: Duration,
    clocks: &SetClocks,
) -> (
    ReferencedBoardClock,
    Arc<AtomicUsize>,
    Arc<Mutex<Vec<String>>>,
) {
    let reads = Arc::new(AtomicUsize::new(0));
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let heard = Arc::clone(&warnings);
    let source = Fixed {
        time,
        reads: Arc::clone(&reads),
    };
    let clock = ReferencedBoardClock::new(Ok(Arc::new(source)), every, move |why| {
        heard.lock().unwrap().push(why.to_string())
    })
    .with_local_clocks(clocks.clone());
    (clock, reads, warnings)
}

fn spelt(time: SystemTime) -> Timestamp {
    timestamp_at(time).unwrap()
}

#[test]
fn the_reference_advances_with_the_monotonic_clock_and_is_read_once_per_period() {
    let clocks = SetClocks::new();
    let outside = clocks.wall() - Duration::from_secs(600);
    let (clock, reads, _) = clock_of(Ok(outside), Duration::from_secs(3600), &clocks);
    assert_eq!(clock.reference(), Some(spelt(outside)));
    clocks.advance(Duration::from_secs(100), true);
    assert_eq!(
        clock.reference(),
        Some(spelt(outside + Duration::from_secs(100)))
    );
    assert_eq!(reads.load(Ordering::Relaxed), 1, "kept for the period");
    clocks.advance(Duration::from_secs(3600), true);
    clock.reference();
    assert_eq!(reads.load(Ordering::Relaxed), 2, "read again once old");
    assert_eq!(clock.reference_status(), ReferenceStatus::Referenced);
}

#[test]
fn a_suspend_makes_the_reading_stale_and_it_is_read_again() {
    let clocks = SetClocks::new();
    let outside = clocks.wall();
    let (clock, reads, _) = clock_of(Ok(outside), Duration::from_secs(3600), &clocks);
    clock.reference();
    clocks.advance(SUSPEND_TOLERANCE, false);
    clock.reference();
    assert_eq!(reads.load(Ordering::Relaxed), 1, "within the tolerance");
    clocks.advance(Duration::from_secs(600), false);
    clock.reference();
    assert_eq!(
        reads.load(Ordering::Relaxed),
        2,
        "the clocks disagree: read again"
    );
}

#[test]
fn a_failure_stands_for_the_period_says_why_and_is_warned_about_once() {
    let clocks = SetClocks::new();
    let (clock, reads, warnings) = clock_of(
        Err("no route to host".into()),
        Duration::from_secs(3600),
        &clocks,
    );
    for _ in 0..3 {
        assert_eq!(clock.reference(), None);
    }
    assert_eq!(
        reads.load(Ordering::Relaxed),
        1,
        "one failed read per period"
    );
    assert_eq!(
        clock.reference_status(),
        ReferenceStatus::NoReference("no route to host".into())
    );
    assert!(clock.now().is_ok(), "the clock still tells the time");
    assert_eq!(*warnings.lock().unwrap(), ["no route to host"]);
}

#[test]
fn an_outside_time_a_day_away_is_not_believed() {
    let clocks = SetClocks::new();
    let far = clocks.wall() + MAX_PLAUSIBLE_OFFSET + Duration::from_secs(60);
    let (clock, _, warnings) = clock_of(Ok(far), Duration::from_secs(3600), &clocks);
    assert_eq!(clock.reference(), None);
    assert!(matches!(
        clock.reference_status(),
        ReferenceStatus::NoReference(_)
    ));
    assert_eq!(warnings.lock().unwrap().len(), 1);
}

/// A source that answers only when released, after telling it was asked.
struct Held {
    asked: Mutex<mpsc::Sender<()>>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl TimeReading for Held {
    fn read(&self) -> Result<SystemTime, String> {
        let _ = self.asked.lock().unwrap().send(());
        let _ = self.release.lock().unwrap().recv();
        Ok(SystemTime::now())
    }
}

#[test]
fn one_reading_runs_at_a_time_and_none_outlasts_the_read_timeout() {
    let (asked, was_asked) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let source = Held {
        asked: Mutex::new(asked),
        release: Mutex::new(released),
    };
    let clock = Arc::new(
        ReferencedBoardClock::new(Ok(Arc::new(source)), Duration::from_secs(3600), |_| {})
            .with_read_timeout(Duration::from_millis(1500)),
    );
    let first = {
        let clock = Arc::clone(&clock);
        std::thread::spawn(move || clock.reference())
    };
    was_asked.recv().unwrap();
    let started = Instant::now();
    assert_eq!(
        clock.reference(),
        None,
        "a reading in flight is not waited for"
    );
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(first.join().unwrap(), None, "abandoned at the read timeout");
    let _ = release.send(());
}

#[test]
fn a_remote_with_no_reference_says_why() {
    let clock = ReferencedBoardClock::for_remote("/srv/git/repo.git");
    assert_eq!(clock.reference(), None);
    let ReferenceStatus::NoReference(why) = clock.reference_status() else {
        panic!("no reference")
    };
    assert!(why.contains("neither"), "{why}");
}
