//! The stream driver (epic #2705 B10) on its own: a caller whose blocking
//! answers are all held still has its reader's going noticed at once, and
//! still gets its keep-alives.
use std::time::Duration;

use super::*;
use crate::infrastructure::http::board_service::sources::Streams;

/// A source that never has anything to send.
struct Idle;

impl ServiceStream for Idle {
    fn next(&mut self) -> StreamStep {
        StreamStep::Send {
            bytes: vec![],
            more: false,
        }
    }

    fn content_type(&self) -> &'static str {
        "text/event-stream"
    }

    fn keepalive(&self) -> &'static [u8] {
        b": ping\n\n"
    }
}

/// A stream of principal `a`, all of whose 4 blocking answers are held
/// (the permits are given back to keep them held), and its streams' cap of
/// one; pinged each `ping`.
fn starved(
    ping: Duration,
) -> (
    Streamed,
    Driven,
    Arc<Streams>,
    Vec<[tokio::sync::OwnedSemaphorePermit; 2]>,
) {
    let answers = Arc::new(Answers::new(12, 4));
    let held: Vec<_> = (0..4)
        .map(|_| answers.admit("a").expect("a free answer"))
        .collect();
    assert!(answers.admit("a").is_none(), "every answer of a's is held");
    let streams = Arc::new(Streams::new(32, 1));
    let place = streams.take(Some("a")).expect("a's one stream place");
    let streamed = Streamed {
        source: Box::new(Idle),
        woken: Arc::new(Notify::new()),
        principal: Some("a".into()),
        place,
    };
    let driven = Driven {
        answers,
        ends: tokio::time::Instant::now() + Duration::from_secs(30),
        ping,
    };
    (streamed, driven, streams, held)
}

/// Review (#2756 L3): with every answer of its caller held, a reader that
/// goes ends the driver within about 100 ms, and its place is free again.
#[tokio::test]
async fn a_starved_stream_ends_when_its_reader_goes() {
    let (streamed, driven, streams, _held) = starved(Duration::from_secs(15));
    let (out, taken) = mpsc::channel::<Bytes>(limits::STREAM_CHUNKS);
    let driver = tokio::spawn(pump(streamed, out, driven));
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert!(
        streams.take(Some("a")).is_none(),
        "its place is held while it runs"
    );
    drop(taken);
    let ended = tokio::time::timeout(Duration::from_millis(100), driver).await;
    assert!(ended.is_ok(), "the driver ends as its reader goes");
    assert!(streams.take(Some("a")).is_some(), "its place is free again");
}

/// Review (#2756 L4): with every answer of its caller held, the driver
/// still sends a keep-alive each ping, so the reader never times out.
#[tokio::test]
async fn a_starved_stream_still_sends_its_keepalives() {
    let (streamed, driven, _streams, _held) = starved(Duration::from_millis(100));
    let (out, mut taken) = mpsc::channel::<Bytes>(limits::STREAM_CHUNKS);
    let driver = tokio::spawn(pump(streamed, out, driven));
    let mut pings = 0;
    let began = tokio::time::Instant::now();
    while began.elapsed() < Duration::from_millis(450) {
        let wait = Duration::from_millis(450).saturating_sub(began.elapsed());
        match tokio::time::timeout(wait, taken.recv()).await {
            Ok(Some(bytes)) if bytes.as_ref() == b": ping\n\n" => pings += 1,
            Ok(Some(other)) => panic!("only keep-alives: {other:?}"),
            Ok(None) | Err(_) => break,
        }
    }
    assert!(
        pings >= 3,
        "a keep-alive each 100 ms while starved: {pings}"
    );
    drop(taken);
    driver.abort();
}
