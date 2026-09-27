//! #1942 ledger rows 3-5: redirects are followed by the adapter itself, for
//! exactly reqwest's statuses, only on exactly one valid `Location`, and
//! under one deadline for the whole fetch. Local listeners only.
use super::*;
use crate::application::agent_turn::use_cases::web_fetch::ParsedHttpUrl;
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::Duration,
};

fn request(url: &str) -> FetchRequest {
    FetchRequest {
        url: ParsedHttpUrl::parse(url).unwrap(),
    }
}

/// A listener that answers its connections with `responses` in turn (the
/// last one repeated), after `delay` each, recording every request head.
async fn scripted_peer(
    responses: Vec<String>,
    delay: Duration,
) -> (u16, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let task = tokio::spawn({
        let seen = seen.clone();
        async move {
            for index in 0.. {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 4096];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                seen.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buffer[..read]).to_ascii_lowercase());
                tokio::time::sleep(delay).await;
                let response = &responses[index.min(responses.len() - 1)];
                let _ = socket.write_all(response.as_bytes()).await;
            }
        }
    });
    (port, seen, task)
}

fn redirect(status: u16, locations: &[&str]) -> String {
    let headers: String = locations
        .iter()
        .map(|location| format!("Location: {location}\r\n"))
        .collect();
    format!("HTTP/1.1 {status} Redirect\r\nConnection: close\r\n{headers}Content-Length: 0\r\n\r\n")
}

fn ok_body(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

/// A target listener counting the connections it accepted.
async fn target() -> (u16, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
    scripted_peer(vec![ok_body("target")], Duration::ZERO).await
}

fn adapter() -> ReqwestFetchWebContent {
    ReqwestFetchWebContent::with_policy(LOOPBACK_FOR_TESTS)
}

#[tokio::test]
async fn each_followed_status_reaches_the_target_once_with_a_get_and_referer() {
    for status in REDIRECT_STATUSES {
        let (target_port, target_seen, target_task) = target().await;
        let location = format!("http://localhost:{target_port}/final");
        let (port, seen, task) =
            scripted_peer(vec![redirect(status, &[&location])], Duration::ZERO).await;
        let result = adapter()
            .fetch(&request(&format!("http://localhost:{port}/start#part")))
            .await;
        assert_eq!(
            result,
            Ok(FetchOutcome::SuccessBody {
                body: b"target".to_vec(),
                content_type: None
            }),
            "{status}"
        );
        assert_eq!(seen.lock().unwrap().len(), 1, "{status}");
        let hops = target_seen.lock().unwrap().clone();
        assert_eq!(hops.len(), 1, "{status}");
        assert!(
            hops[0].starts_with("get /final http/1.1"),
            "{status}: {}",
            hops[0]
        );
        assert!(
            hops[0].contains(&format!("referer: http://localhost:{port}/start\r\n")),
            "{status}: {}",
            hops[0]
        );
        assert!(
            hops[0].contains(&format!("user-agent: quecto/{}", env!("CARGO_PKG_VERSION"))),
            "{status}"
        );
        task.abort();
        target_task.abort();
    }
}

#[tokio::test]
async fn other_three_hundred_statuses_are_returned_and_never_followed() {
    for status in [300, 304, 305, 306] {
        let (target_port, target_seen, target_task) = target().await;
        let location = format!("http://localhost:{target_port}/final");
        let (port, _, task) =
            scripted_peer(vec![redirect(status, &[&location])], Duration::ZERO).await;
        let result = adapter()
            .fetch(&request(&format!("http://localhost:{port}/start")))
            .await;
        match result {
            Ok(FetchOutcome::NonSuccessStatus(returned)) => assert_eq!(returned.code, status),
            other => panic!("{status}: expected the status itself, got {other:?}"),
        }
        assert!(target_seen.lock().unwrap().is_empty(), "{status}");
        task.abort();
        target_task.abort();
    }
}

/// Ledger row 5: no Location, two (even agreeing), an empty or an invalid
/// one fails closed before anything is sent to any target.
#[tokio::test]
async fn a_redirect_without_exactly_one_valid_location_fails_closed_unsent() {
    let (target_port, target_seen, target_task) = target().await;
    let good = format!("http://localhost:{target_port}/final");
    for (locations, why) in [
        (vec![], "0 Location headers, not exactly one"),
        (
            vec![good.as_str(), good.as_str()],
            "2 Location headers, not exactly one",
        ),
        (
            vec![good.as_str(), "http://localhost:1/other"],
            "2 Location headers, not exactly one",
        ),
        (vec![""], "its Location is not a valid URL"),
        (vec!["   "], "its Location is not a valid URL"),
        (vec!["http://[::1"], "its Location is not a valid URL"),
        (
            vec!["http://exa mple.com/"],
            "its Location is not a valid URL",
        ),
    ] {
        let (port, _, task) = scripted_peer(vec![redirect(302, &locations)], Duration::ZERO).await;
        let result = adapter()
            .fetch(&request(&format!("http://localhost:{port}/start")))
            .await;
        assert_eq!(
            result,
            Err(FetchFailure::BadRedirect(format!(
                "the redirect from http://localhost:{port}/start: {why}"
            ))),
            "{locations:?}"
        );
        task.abort();
    }
    assert!(target_seen.lock().unwrap().is_empty(), "no target request");
    target_task.abort();
}

/// Ledger row 3: one deadline covers the whole fetch. Three hops of 250 ms
/// each fit no 600 ms deadline, though every hop alone would.
#[tokio::test]
async fn one_deadline_covers_every_redirect() {
    let (port, seen, task) = scripted_peer(
        vec![
            redirect(302, &["/again"]),
            redirect(302, &["/again"]),
            ok_body("late"),
        ],
        Duration::from_millis(250),
    )
    .await;
    let adapter = adapter().with_timeout(Duration::from_millis(600));
    let started = std::time::Instant::now();
    let result = adapter
        .fetch(&request(&format!("http://localhost:{port}/start")))
        .await;
    assert_eq!(result, Err(FetchFailure::TimedOut));
    assert!(
        started.elapsed() < Duration::from_millis(900),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(seen.lock().unwrap().len(), 3, "the third hop was cut short");
    task.abort();
    // The same hops under a deadline that fits them all succeed.
    let (port, _, task) = scripted_peer(
        vec![
            redirect(302, &["/again"]),
            redirect(302, &["/again"]),
            ok_body("late"),
        ],
        Duration::from_millis(50),
    )
    .await;
    let result = ReqwestFetchWebContent::with_policy(LOOPBACK_FOR_TESTS)
        .with_timeout(Duration::from_millis(600))
        .fetch(&request(&format!("http://localhost:{port}/start")))
        .await;
    assert!(
        matches!(result, Ok(FetchOutcome::SuccessBody { .. })),
        "{result:?}"
    );
    task.abort();
}

#[test]
fn a_referer_drops_credentials_and_fragment_and_is_never_sent_from_https_to_http() {
    let url = |text: &str| url::Url::parse(text).unwrap();
    assert_eq!(
        referer(&url("http://b.test/"), &url("http://u:p@a.test/x?q#f")).unwrap(),
        "http://a.test/x?q"
    );
    assert_eq!(
        referer(&url("https://b.test/"), &url("http://a.test/")).unwrap(),
        "http://a.test/"
    );
    assert_eq!(
        referer(&url("http://b.test/"), &url("https://a.test/")),
        None
    );
}

#[test]
fn only_the_five_redirect_statuses_are_followed() {
    let base = url::Url::parse("http://a.test/x").unwrap();
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::LOCATION, "/y".parse().unwrap());
    for status in 100..600 {
        let followed = redirect_target(status, &headers, &base).unwrap();
        assert_eq!(
            followed.is_some(),
            [301, 302, 303, 307, 308].contains(&status),
            "{status}"
        );
    }
    assert_eq!(
        redirect_target(302, &headers, &base)
            .unwrap()
            .unwrap()
            .as_str(),
        "http://a.test/y"
    );
}
