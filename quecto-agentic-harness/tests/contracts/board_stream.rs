//! Contract for [`BoardStream`] (epic #2705 B10), on the remote feed's
//! open streams over canned replies: each event decoded once; an end, or a
//! cut in transport (a clean close at a chunk boundary among them), is
//! `Ended`; a stream this client does not read is `Malformed`; `retry:` is
//! the stream's.
use std::time::Duration;

use quecto::application::project_board::ports::feed::{BoardStreams, StreamRead};

use super::board_streams::{canned, reads, revision, sse};

fn read_all(reply: Vec<u8>) -> (Vec<StreamRead>, Option<Duration>) {
    let (_dir, streams, _) = canned(vec![reply]);
    let mut stream = streams
        .open(Some(0))
        .unwrap_or_else(|why| panic!("{why:?}"));
    let read = reads(&mut stream);
    (read, stream.retry())
}

#[test]
fn an_ended_stream_is_ended_after_its_events() {
    let body = format!("retry: 2500\n\n{}{}", revision(1), revision(2));
    let (read, retry) = read_all(sse(&body, true));
    assert_eq!(read.len(), 3, "{read:?}");
    assert_eq!(read[2], StreamRead::Ended);
    assert_eq!(retry, Some(Duration::from_millis(2_500)));
}

/// Review (#2757 round 2, M1): a connection closed cleanly at a chunk
/// boundary, with no last chunk, is a cut, `Ended`: the watch reopens.
#[test]
fn a_clean_close_at_a_chunk_boundary_is_ended() {
    let (read, _) = read_all(sse(&revision(1), false));
    assert_eq!(read.len(), 2, "{read:?}");
    assert_eq!(read[1], StreamRead::Ended);
}

/// A stream this client does not read (an unknown field; an event off the
/// schema, text riding along) is malformed, saying so.
#[test]
fn a_stream_off_the_schema_is_malformed() {
    let off = "id: 4\nevent: revision\n\
               data: {\"kind\":\"revision\",\"epoch\":1,\"revision\":4,\"title\":\"x\"}\n\n";
    for (body, said) in [("title: x\n\n", "malformed"), (off, "does not read")] {
        let (read, _) = read_all(sse(body, true));
        let malformed = matches!(&read[..], [StreamRead::Malformed(why)] if why.contains(said));
        assert!(malformed, "{body:?}: {read:?}");
    }
}
