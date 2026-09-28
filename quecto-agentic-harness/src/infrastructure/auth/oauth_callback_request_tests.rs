// Tests for the OAuth callback listener's request framing (PR #2309).

use super::*;
use std::time::Duration;

fn later(secs: u64) -> Instant {
    Instant::now() + Duration::from_secs(secs)
}

/// Content-Length of a canned response must match its body exactly.
fn assert_content_length_matches(response: &str) {
    let (head, body) = response.split_once("\r\n\r\n").expect("head and body");
    let declared: usize = head
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .expect("Content-Length header")
        .parse()
        .expect("numeric Content-Length");
    assert_eq!(declared, body.len(), "{response:?}");
}

#[test]
fn canned_responses_declare_their_body_length() {
    assert_content_length_matches(HEAD_TOO_LARGE_RESPONSE);
    assert_content_length_matches(INCOMPLETE_REQUEST_RESPONSE);
    assert!(HEAD_TOO_LARGE_RESPONSE.starts_with("HTTP/1.1 431 "));
    assert!(INCOMPLETE_REQUEST_RESPONSE.starts_with("HTTP/1.1 400 "));
}

#[tokio::test]
async fn head_is_returned_through_its_blank_line_only() {
    let mut input: &[u8] = b"GET /cb?a=1 HTTP/1.1\r\nHost: x\r\n\r\ntrailing body";
    let head = read_request_head(&mut input, later(5)).await;
    assert_eq!(
        head,
        RequestHead::Complete("GET /cb?a=1 HTTP/1.1\r\nHost: x\r\n\r\n".to_string())
    );
}

#[tokio::test]
async fn terminator_split_across_reads_is_found() {
    let (mut client, mut server) = tokio::io::duplex(64);
    let writer = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt;
        for piece in [&b"GET / HTTP/1.1\r\n\r"[..], b"\n"] {
            client.write_all(piece).await.unwrap();
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        client
    });
    let head = read_request_head(&mut server, later(5)).await;
    assert_eq!(
        head,
        RequestHead::Complete("GET / HTTP/1.1\r\n\r\n".to_string())
    );
    drop(writer.await.unwrap());
}

#[tokio::test]
async fn head_of_exactly_the_cap_is_accepted_and_one_byte_more_is_not() {
    let prefix = "GET / HTTP/1.1\r\nX-Pad: ";
    let suffix = "\r\n\r\n";
    let pad = MAX_REQUEST_HEAD_BYTES - prefix.len() - suffix.len();
    let at_cap = format!("{prefix}{}{suffix}", "p".repeat(pad));
    assert_eq!(at_cap.len(), MAX_REQUEST_HEAD_BYTES);
    let mut input = at_cap.as_bytes();
    assert_eq!(
        read_request_head(&mut input, later(5)).await,
        RequestHead::Complete(at_cap.clone())
    );

    let over_cap = format!("{prefix}{}{suffix}", "p".repeat(pad + 1));
    let mut input = over_cap.as_bytes();
    assert_eq!(
        read_request_head(&mut input, later(5)).await,
        RequestHead::Rejected(HEAD_TOO_LARGE_RESPONSE)
    );
}

#[tokio::test]
async fn endless_head_without_terminator_is_rejected_at_the_cap() {
    let endless = vec![b'x'; MAX_REQUEST_HEAD_BYTES * 4];
    let mut input = endless.as_slice();
    assert_eq!(
        read_request_head(&mut input, later(5)).await,
        RequestHead::Rejected(HEAD_TOO_LARGE_RESPONSE)
    );
    // It stopped reading shortly past the cap rather than consuming it all.
    assert!(input.len() >= endless.len() - MAX_REQUEST_HEAD_BYTES - READ_CHUNK_BYTES);
}

#[tokio::test]
async fn end_of_stream_before_the_blank_line_is_incomplete() {
    for partial in [&b""[..], b"GET ", b"GET /cb HTTP/1.1\r\nHost: x\r\n"] {
        let mut input = partial;
        assert_eq!(
            read_request_head(&mut input, later(5)).await,
            RequestHead::Rejected(INCOMPLETE_REQUEST_RESPONSE),
            "{partial:?}"
        );
    }
}

#[tokio::test]
async fn silent_client_times_out_at_the_deadline() {
    let (_client, mut server) = tokio::io::duplex(64);
    let deadline = Instant::now() + Duration::from_millis(100);
    assert_eq!(
        read_request_head(&mut server, deadline).await,
        RequestHead::TimedOut
    );
}

#[tokio::test]
async fn reject_writes_the_response_and_half_closes() {
    let (mut client, mut server) = tokio::io::duplex(1024);
    let rejecting = tokio::spawn(async move {
        reject(
            &mut server,
            INCOMPLETE_REQUEST_RESPONSE,
            later(5),
            &CallbackLimits::PRODUCTION,
        )
        .await;
    });
    let mut answer = String::new();
    {
        use tokio::io::AsyncReadExt;
        client.read_to_string(&mut answer).await.unwrap();
    }
    assert_eq!(answer, INCOMPLETE_REQUEST_RESPONSE);
    drop(client);
    tokio::time::timeout(Duration::from_secs(5), rejecting)
        .await
        .expect("reject returns once the client goes away")
        .unwrap();
}
