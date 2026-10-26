//! The event-stream reader (epic #2705 B10) against canned replies on a
//! temp socket: chunks split anywhere, every bound held.
use super::*;
use std::os::unix::net::UnixListener;
use std::sync::mpsc;

/// Serves `reply` once on a temp socket; gives the client and the head
/// the client sent.
fn canned(
    reply: Vec<u8>,
) -> (
    tempfile::TempDir,
    BoardServiceClient,
    mpsc::Receiver<String>,
) {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("board.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let (sent, heard) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap() == 1 {
            head.push(byte[0]);
        }
        sent.send(String::from_utf8(head).unwrap()).unwrap();
        stream.write_all(&reply).unwrap();
    });
    (dir, BoardServiceClient::unix(&socket), heard)
}

/// A chunked `text/event-stream` reply whose body is `body`, cut into
/// chunks of `size` bytes.
fn chunked(body: &str, size: usize) -> Vec<u8> {
    let mut reply = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                      transfer-encoding: chunked\r\n\r\n"
        .to_vec();
    for chunk in body.as_bytes().chunks(size) {
        reply.extend(format!("{:x}\r\n", chunk.len()).as_bytes());
        reply.extend(chunk);
        reply.extend(b"\r\n");
    }
    reply.extend(b"0\r\n\r\n");
    reply
}

fn opened(reply: Vec<u8>) -> (Result<Opened, ClientError>, String) {
    let (_dir, client, heard) = canned(reply);
    let opened = client.open_events(
        "/v1/boards/0123456789abcdef/events",
        Some(7),
        Duration::from_secs(5),
    );
    let head = heard.recv_timeout(Duration::from_secs(5)).unwrap();
    (opened, head)
}

const BODY: &str = "retry: 1000\n\n: ping\n\nid: 3\nevent: epoch\ndata: {\"kind\":\"epoch\",\"epoch\":1}\n\n\
                    id: 4\nevent: revision\ndata: {\"kind\":\"revision\",\"epoch\":1,\"revision\":4}\n\n";

#[test]
fn events_are_read_across_any_chunking_until_the_stream_ends() {
    for size in [1, 3, 7, BODY.len()] {
        let (opened, head) = opened(chunked(BODY, size));
        assert!(head.contains("last-event-id: 7\r\n"), "{head}");
        assert!(head.starts_with("GET /v1/boards/0123456789abcdef/events HTTP/1.1\r\n"));
        let Ok(Opened::Stream(mut reader)) = opened else {
            panic!("a stream opens");
        };
        let first = reader.next_event().unwrap().unwrap();
        assert_eq!(
            first,
            StreamEvent {
                id: Some("3".into()),
                event: "epoch".into(),
                data: "{\"kind\":\"epoch\",\"epoch\":1}".into(),
            },
            "chunks of {size}"
        );
        let second = reader.next_event().unwrap().unwrap();
        assert_eq!(
            (second.id.as_deref(), second.event.as_str()),
            (Some("4"), "revision")
        );
        assert_eq!(reader.next_event().unwrap(), None, "the last chunk ends it");
    }
}

#[test]
fn a_refusal_is_the_servers_reply() {
    let body = r#"{"error":"busy","code":"busy"}"#;
    let reply = format!(
        "HTTP/1.1 503 Service Unavailable\r\ncontent-type: application/json\r\n\
         content-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let Ok(Opened::Refused(refused)) = opened(reply.into_bytes()).0 else {
        panic!("a refusal is read whole");
    };
    assert_eq!(
        (refused.status, refused.body),
        (503, body.as_bytes().to_vec())
    );
}

#[test]
fn a_reply_that_is_no_chunked_event_stream_is_malformed() {
    let reply = b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\r\n{}";
    assert!(matches!(
        opened(reply.to_vec()).0,
        Err(ClientError::Malformed(_))
    ));
}

#[test]
fn an_event_past_its_bound_or_an_unknown_field_is_malformed() {
    for body in [
        format!("data: {}\n\n", "x".repeat(MAX_EVENT_BYTES)),
        "title: secret\n\n".to_string(),
    ] {
        let Ok(Opened::Stream(mut reader)) = opened(chunked(&body, 512)).0 else {
            panic!("a stream opens");
        };
        assert!(
            matches!(reader.next_event(), Err(ClientError::Malformed(_))),
            "{body:.40}"
        );
    }
}

#[test]
fn an_event_cut_short_is_dropped() {
    let Ok(Opened::Stream(mut reader)) = opened(chunked("id: 4\nevent: revision\ndata: {", 4)).0
    else {
        panic!("a stream opens");
    };
    assert_eq!(reader.next_event().unwrap(), None);
}

/// Review (#2757 M1): a bad chunk size is a malformed stream, not a
/// transport failure a watch would quietly retry.
#[test]
fn a_bad_chunk_size_is_malformed() {
    let mut reply = chunked("", 1);
    reply.truncate(reply.len() - 5);
    reply.extend(b"zz\r\n");
    let Ok(Opened::Stream(mut reader)) = opened(reply).0 else {
        panic!("a stream opens");
    };
    assert!(matches!(
        reader.next_event(),
        Err(ClientError::Malformed(_))
    ));
}

/// Every event a stream of `body` gives, to its end.
fn all_events(body: &str) -> Vec<StreamEvent> {
    let Ok(Opened::Stream(mut reader)) = opened(chunked(body, 5)).0 else {
        panic!("a stream opens");
    };
    std::iter::from_fn(|| reader.next_event().unwrap()).collect()
}

/// Review (#2757 L3, L4): lines end in LF, CRLF or a bare CR alike; data
/// over several lines is joined by LF.
#[test]
fn lines_end_in_lf_crlf_or_cr_and_data_lines_join() {
    for body in [
        "id: 4\nevent: revision\ndata: a\ndata: b\n\n",
        "id: 4\r\nevent: revision\r\ndata: a\r\ndata: b\r\n\r\n",
        "id: 4\revent: revision\rdata: a\rdata: b\r\r",
    ] {
        let events = all_events(body);
        let want = StreamEvent {
            id: Some("4".into()),
            event: "revision".into(),
            data: "a\nb".into(),
        };
        assert_eq!(events, vec![want], "{body:?}");
    }
}

/// Review (#2757 L3): an event with no data is not dispatched, as the SSE
/// spec has it; the next one is.
#[test]
fn an_event_with_no_data_is_not_dispatched() {
    let events = all_events("id: 3\nevent: epoch\n\nid: 4\nevent: revision\ndata: x\n\n");
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].id.as_deref(), Some("4"));
}

/// Review (#2757 L3): a stream's `retry:` is the reader's reconnection
/// delay, up to a minute; one not a number is ignored.
#[test]
fn retry_sets_the_reconnection_delay() {
    for (body, retry) in [
        ("retry: 2500\n\n", Some(Duration::from_millis(2_500))),
        ("retry: soon\n\n", None),
        ("retry: 999999999\n\n", Some(MAX_RETRY)),
    ] {
        let Ok(Opened::Stream(mut reader)) = opened(chunked(body, 5)).0 else {
            panic!("a stream opens");
        };
        assert_eq!(reader.next_event().unwrap(), None);
        assert_eq!(reader.retry(), retry, "{body:?}");
    }
}
