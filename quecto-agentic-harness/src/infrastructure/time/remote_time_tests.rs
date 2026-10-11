use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

#[test]
fn https_remotes_are_asked_at_their_own_host_and_ssh_only_at_a_known_forge() {
    let https = |url: &str| ReferenceSource::Https(url.into());
    for (remote, source) in [
        (
            "https://github.com/platform-q-ai/quecto.git",
            https("https://github.com/"),
        ),
        (
            "https://user:token@Git.Example.org:8443/a/b.git",
            https("https://git.example.org:8443/"),
        ),
        (
            "git@github.com:platform-q-ai/quecto.git",
            https("https://github.com/"),
        ),
        (
            "ssh://git@GitLab.com:22/group/repo.git",
            https("https://gitlab.com/"),
        ),
        (
            "ssh://git@github.com/org/repo",
            https("https://github.com/"),
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
        "https:///repo",
        "",
    ] {
        assert!(
            matches!(reference_source(remote), ReferenceSource::Unavailable(_)),
            "{remote}"
        );
    }
}

/// A one-request HTTP server answering with `date` as its `Date` header;
/// its URL.
fn serves_date(date: SystemTime) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 1024];
        let _ = stream.read(&mut request);
        let answer = format!(
            "HTTP/1.1 200 OK\r\nDate: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            httpdate::fmt_http_date(date)
        );
        stream.write_all(answer.as_bytes()).unwrap();
    });
    url
}

fn closed_port() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    drop(listener);
    url
}

#[test]
fn the_date_header_is_read_and_an_unreachable_host_is_an_error() {
    let hour_ago = SystemTime::now() - Duration::from_secs(3600);
    let read = HttpDate::new(serves_date(hour_ago)).read().unwrap();
    let off = hour_ago
        .duration_since(read)
        .unwrap_or_else(|ahead| ahead.duration());
    assert!(off < Duration::from_secs(1), "{off:?}");
    let unreachable = HttpDate::new(closed_port())
        .with_timeout(Duration::from_secs(2))
        .read();
    assert!(unreachable.is_err(), "{unreachable:?}");
}

/// A source reporting `offset` from this machine's time, counting reads.
struct Offset {
    offset: i64,
    reads: Arc<AtomicUsize>,
}

impl TimeReading for Offset {
    fn read(&self) -> Result<SystemTime, String> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let delta = Duration::from_secs(self.offset.unsigned_abs());
        Ok(match self.offset >= 0 {
            true => SystemTime::now() + delta,
            false => SystemTime::now() - delta,
        })
    }
}

fn offset_clock(offset: i64, every: Duration) -> (ReferencedBoardClock, Arc<AtomicUsize>) {
    let reads = Arc::new(AtomicUsize::new(0));
    let source = Offset {
        offset,
        reads: reads.clone(),
    };
    (
        ReferencedBoardClock::new(Ok(Box::new(source)), every, |_| {}),
        reads,
    )
}

#[test]
fn the_reference_is_read_once_per_period_and_advances_with_the_local_clock() {
    let (clock, reads) = offset_clock(-600, Duration::from_secs(3600));
    let first = clock.reference().unwrap();
    let second = clock.reference().unwrap();
    assert_eq!(reads.load(Ordering::Relaxed), 1, "kept for the period");
    assert!(first <= second);
    let expected = timestamp_at(SystemTime::now() - Duration::from_secs(600)).unwrap();
    let skew = expected.as_str().cmp(second.as_str());
    assert!(skew != std::cmp::Ordering::Less, "{second:?} {expected:?}");
    let (stale, reads) = offset_clock(0, Duration::ZERO);
    stale.reference();
    stale.reference();
    assert_eq!(reads.load(Ordering::Relaxed), 2, "read again once stale");
}

#[test]
fn no_reference_is_not_a_failure_and_is_warned_about_once() {
    let warnings = Arc::new(Mutex::new(Vec::<String>::new()));
    let heard = warnings.clone();
    let unreachable = HttpDate::new(closed_port()).with_timeout(Duration::from_secs(2));
    let clock = ReferencedBoardClock::new(Ok(Box::new(unreachable)), Duration::ZERO, move |why| {
        heard.lock().unwrap().push(why.into())
    });
    assert_eq!(clock.reference(), None);
    assert_eq!(clock.reference(), None);
    assert!(clock.now().is_ok(), "the clock still tells the time");
    assert_eq!(warnings.lock().unwrap().len(), 1, "{warnings:?}");
    let silent = Arc::new(Mutex::new(Vec::<String>::new()));
    let heard = silent.clone();
    let ssh = match reference_source("git@git.example.org:repo.git") {
        ReferenceSource::Unavailable(why) => why,
        other => panic!("{other:?}"),
    };
    let clock = ReferencedBoardClock::new(Err(ssh), REFRESH_EVERY, move |why| {
        heard.lock().unwrap().push(why.into())
    });
    assert_eq!((clock.reference(), clock.reference()), (None, None));
    let said = silent.lock().unwrap();
    assert!(
        said.len() == 1 && said[0].contains("not a known forge"),
        "{said:?}"
    );
}
