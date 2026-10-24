use super::*;

#[test]
fn only_get_and_post_are_admitted() {
    assert_eq!(admit("GET", &[], false).unwrap().method, Method::Get);
    assert_eq!(admit("POST", &[], false).unwrap().method, Method::Post);
    for method in ["get", "PUT", "DELETE", "HEAD", "OPTIONS", "BREW", ""] {
        assert_eq!(admit(method, &[], false), Err(Refusal::Method), "{method}");
    }
}

#[test]
fn a_body_is_admitted_only_by_one_plain_length_up_to_one_mebibyte() {
    let most = MAX_BODY_BYTES.to_string();
    assert_eq!(
        admit("POST", &[most.as_bytes()], false).unwrap().body_bytes,
        MAX_BODY_BYTES
    );
    let over = (MAX_BODY_BYTES + 1).to_string();
    assert_eq!(
        admit("POST", &[over.as_bytes()], false),
        Err(Refusal::TooLarge)
    );
    for bad in [
        &b""[..],
        b"-1",
        b"+1",
        b"1 ",
        b"0x10",
        b"99999999999999999999",
    ] {
        assert_eq!(admit("POST", &[bad], false), Err(Refusal::BadLength));
    }
    assert_eq!(admit("POST", &[b"1", b"1"], false), Err(Refusal::BadLength));
    assert_eq!(admit("POST", &[b"1"], true), Err(Refusal::LengthRequired));
}

#[test]
fn each_refusal_has_its_status() {
    let statuses = [
        Refusal::Method,
        Refusal::LengthRequired,
        Refusal::BadLength,
        Refusal::TooLarge,
    ]
    .map(|refusal| refusal.verdict().0);
    assert_eq!(statuses, [405, 411, 400, 413]);
}

/// Review round 2: a peer that keeps taking bytes, each pause shorter
/// than the limit but all of them longer, never times out: progress
/// restarts the clock.
#[tokio::test(start_paused = true)]
async fn progress_restarts_the_write_clock() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (near, mut far) = tokio::io::duplex(8);
    let mut writer = WriteDeadline::new(near, Duration::from_millis(100));
    let writing = tokio::spawn(async move { writer.write_all(&[7; 48]).await });
    let mut taken = [0; 48];
    for chunk in taken.chunks_mut(8) {
        tokio::time::sleep(Duration::from_millis(70)).await;
        far.read_exact(chunk).await.unwrap();
    }
    assert!(writing.await.unwrap().is_ok(), "no write timed out");
    assert_eq!(taken, [7; 48]);
}

/// A peer that takes nothing times the write out at the limit.
#[tokio::test(start_paused = true)]
async fn a_peer_that_takes_nothing_times_the_write_out() {
    use tokio::io::AsyncWriteExt;
    let (near, _far) = tokio::io::duplex(8);
    let mut writer = WriteDeadline::new(near, Duration::from_millis(100));
    let error = writer.write_all(&[7; 16]).await.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
}
