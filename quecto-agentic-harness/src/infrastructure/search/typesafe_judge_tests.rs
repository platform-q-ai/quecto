use super::*;
use wiremock::matchers::{body_partial_json, header, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn candidate(location: &str) -> RelevanceCandidate {
    RelevanceCandidate {
        location: location.into(),
        before: String::new(),
        matched: format!("text at {location}"),
        after: String::new(),
    }
}

fn judge(server: &MockServer, timeout: Duration) -> TypeSafeJudge {
    TypeSafeJudge::new(&server.uri(), "test-key".into(), "jev-latest", timeout)
        .expect("the client builds")
}

fn noul(score: f64) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "model": "jev-1.12",
        "answers": {"relevant": {"type": "noul", "noul": score}},
        "usage": {"input_tokens": 10, "output_tokens": 1}
    }))
}

/// Each hit is asked about alone, with the query and its own text, under
/// the key as a bearer token; the scores come back in candidate order.
#[tokio::test]
async fn each_hit_is_judged_alone_and_scores_return_in_order() {
    let server = MockServer::start().await;
    for (location, score) in [("a.rs:1", 0.9), ("b.rs:2", 0.1)] {
        Mock::given(method("POST"))
            .and(header("authorization", "Bearer test-key"))
            .and(body_partial_json(json!({
                "model": "jev-latest",
                "state": {"query": "retry backoff", "candidate": {"location": location}},
                "questions": {"relevant": {"type": "noul"}}
            })))
            .respond_with(noul(score))
            .expect(1)
            .mount(&server)
            .await;
    }
    let candidates = [candidate("a.rs:1"), candidate("b.rs:2")];
    let answer = judge(&server, Duration::from_secs(5))
        .judge("retry backoff", &candidates)
        .await;
    assert_eq!(answer, Relevance::Scored(vec![Some(0.9), Some(0.1)]));
}

/// A hit whose request fails is unscored; when none can be scored the
/// answer says why, so the search returns rg's order with that reason.
#[tokio::test]
async fn failures_leave_hits_unscored_and_all_failing_is_unavailable() {
    let server = MockServer::start().await;
    Mock::given(body_partial_json(
        json!({"state": {"candidate": {"location": "ok.rs:1"}}}),
    ))
    .respond_with(noul(0.7))
    .mount(&server)
    .await;
    Mock::given(body_partial_json(
        json!({"state": {"candidate": {"location": "bad.rs:1"}}}),
    ))
    .respond_with(ResponseTemplate::new(529))
    .mount(&server)
    .await;
    let judged = judge(&server, Duration::from_secs(5))
        .judge("q", &[candidate("ok.rs:1"), candidate("bad.rs:1")])
        .await;
    assert_eq!(
        judged,
        Relevance::Partial(
            vec![Some(0.7), None],
            "TypeSafe is rate-limiting or overloaded (HTTP 529)".into()
        )
    );

    let rejected = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&rejected)
        .await;
    assert_eq!(
        judge(&rejected, Duration::from_secs(5))
            .judge("q", &[candidate("a.rs:1")])
            .await,
        Relevance::Unavailable("TypeSafe rejected the API key (401)".into())
    );

    let malformed = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answers": {}})))
        .mount(&malformed)
        .await;
    assert_eq!(
        judge(&malformed, Duration::from_secs(5))
            .judge("q", &[candidate("a.rs:1")])
            .await,
        Relevance::Unavailable("TypeSafe answered without a relevance score".into())
    );
}

/// Judging is time-bounded as a whole.
#[tokio::test]
async fn judging_stops_at_its_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(noul(0.5).set_delay(Duration::from_secs(10)))
        .mount(&server)
        .await;
    let started = std::time::Instant::now();
    let answer = judge(&server, Duration::from_millis(300))
        .judge("q", &[candidate("a.rs:1")])
        .await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert!(matches!(answer, Relevance::Unavailable(_)), "{answer:?}");
}

#[tokio::test]
async fn nothing_to_judge_is_an_empty_ranking() {
    let server = MockServer::start().await;
    assert_eq!(
        judge(&server, Duration::from_secs(1)).judge("q", &[]).await,
        Relevance::Scored(vec![])
    );
}

/// A score outside 0–1 is not a probability: the hit is left unscored.
#[tokio::test]
async fn a_score_outside_zero_to_one_is_refused() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(noul(1.7))
        .mount(&server)
        .await;
    assert_eq!(
        judge(&server, Duration::from_secs(5))
            .judge("q", &[candidate("a.rs:1")])
            .await,
        Relevance::Unavailable("TypeSafe answered without a relevance score".into())
    );
}

/// Scores that arrive before the deadline are kept; only the late hits are
/// left unscored.
#[tokio::test]
async fn at_the_deadline_the_scores_already_judged_are_kept() {
    let server = MockServer::start().await;
    Mock::given(body_partial_json(
        json!({"state": {"candidate": {"location": "fast.rs:1"}}}),
    ))
    .respond_with(noul(0.8))
    .mount(&server)
    .await;
    Mock::given(body_partial_json(
        json!({"state": {"candidate": {"location": "slow.rs:1"}}}),
    ))
    .respond_with(noul(0.2).set_delay(Duration::from_secs(10)))
    .mount(&server)
    .await;
    let started = std::time::Instant::now();
    let answer = judge(&server, Duration::from_millis(700))
        .judge("q", &[candidate("fast.rs:1"), candidate("slow.rs:1")])
        .await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(
        answer,
        Relevance::Partial(
            vec![Some(0.8), None],
            "TypeSafe did not answer within 700 ms".into()
        )
    );
}

/// Requests run `concurrency` at a time: 32 slow answers at 32 in flight
/// take about one answer's time, not four (the old fixed 8).
#[tokio::test]
async fn requests_run_concurrency_at_a_time() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(noul(0.5).set_delay(Duration::from_millis(400)))
        .mount(&server)
        .await;
    let candidates: Vec<RelevanceCandidate> =
        (0..32).map(|n| candidate(&format!("f{n}.rs:1"))).collect();
    let started = std::time::Instant::now();
    let answer = judge(&server, Duration::from_secs(10))
        .with_concurrency(32)
        .judge("q", &candidates)
        .await;
    assert!(
        started.elapsed() < Duration::from_millis(1200),
        "{:?}",
        started.elapsed()
    );
    assert!(matches!(answer, Relevance::Scored(ref scores) if scores.len() == 32));
}

/// A rejected key stops judging: the rest are never asked (they would only
/// upload code to be refused).
#[tokio::test]
async fn a_rejected_key_stops_judging_at_once() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let candidates: Vec<RelevanceCandidate> =
        (0..200).map(|n| candidate(&format!("f{n}.rs:1"))).collect();
    let answer = judge(&server, Duration::from_secs(10))
        .with_concurrency(4)
        .judge("q", &candidates)
        .await;
    assert_eq!(
        answer,
        Relevance::Unavailable("TypeSafe rejected the API key (401)".into())
    );
    let asked = server.received_requests().await.unwrap().len();
    assert!(asked <= 8, "{asked} of 200 were asked");
}

/// A rejected key is the reason given even when another error came first,
/// and a 403 says the request was refused rather than blaming the key.
#[tokio::test]
async fn a_rejected_key_outranks_other_errors_and_a_403_is_a_refusal() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401))
        .with_priority(2)
        .mount(&server)
        .await;
    let answer = judge(&server, Duration::from_secs(10))
        .with_concurrency(1)
        .judge("q", &[candidate("a.rs:1"), candidate("b.rs:1")])
        .await;
    assert_eq!(
        answer,
        Relevance::Unavailable("TypeSafe rejected the API key (401)".into())
    );

    let refused = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&refused)
        .await;
    assert_eq!(
        judge(&refused, Duration::from_secs(10))
            .judge("q", &[candidate("a.rs:1"), candidate("b.rs:1")])
            .await,
        Relevance::Unavailable("TypeSafe refused the request (403)".into())
    );
}

/// Asked to wait longer than a search can, the hit is given up at once
/// rather than retried early.
#[tokio::test]
async fn a_long_retry_after_gives_up_the_hit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "30"))
        .mount(&server)
        .await;
    let started = std::time::Instant::now();
    assert_eq!(
        judge(&server, Duration::from_secs(10))
            .judge("q", &[candidate("a.rs:1")])
            .await,
        Relevance::Unavailable("TypeSafe asked to retry after 30 s (HTTP 429)".into())
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

/// A rate-limited request is retried (honouring Retry-After) and then
/// judged; one that stays rate-limited is left unscored with the reason.
#[tokio::test]
async fn rate_limits_are_retried_briefly() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(noul(0.6))
        .with_priority(2)
        .mount(&server)
        .await;
    assert_eq!(
        judge(&server, Duration::from_secs(10))
            .judge("q", &[candidate("a.rs:1")])
            .await,
        Relevance::Scored(vec![Some(0.6)])
    );

    let limited = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(529).insert_header("retry-after", "0"))
        .mount(&limited)
        .await;
    assert_eq!(
        judge(&limited, Duration::from_secs(10))
            .judge("q", &[candidate("a.rs:1")])
            .await,
        Relevance::Unavailable("TypeSafe is rate-limiting or overloaded (HTTP 529)".into())
    );
    assert_eq!(
        limited.received_requests().await.unwrap().len(),
        3,
        "one request and two retries"
    );
}

/// Never more than `concurrency` in flight: 8 slow answers two at a time
/// take four rounds.
#[tokio::test]
async fn no_more_than_concurrency_requests_are_in_flight() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(noul(0.5).set_delay(Duration::from_millis(300)))
        .mount(&server)
        .await;
    let candidates: Vec<RelevanceCandidate> =
        (0..8).map(|n| candidate(&format!("f{n}.rs:1"))).collect();
    let started = std::time::Instant::now();
    judge(&server, Duration::from_secs(10))
        .with_concurrency(2)
        .judge("q", &candidates)
        .await;
    assert!(
        started.elapsed() >= Duration::from_millis(1150),
        "{:?}",
        started.elapsed()
    );
}

/// #2144: the matched lines and their context reach TypeSafe as separate
/// fields, and the question says which is judged.
#[tokio::test]
async fn the_matched_lines_and_their_context_are_sent_apart() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({
            "state": {"candidate": {
                "location": "retry.rs:4",
                "before": "impl Policy {",
                "matched": "/// The delay before the next attempt.",
                "after": "fn backoff_delay(&self) -> Duration {",
            }}
        })))
        .respond_with(noul(0.9))
        .mount(&server)
        .await;
    let candidate = RelevanceCandidate {
        location: "retry.rs:4".into(),
        before: "impl Policy {".into(),
        matched: "/// The delay before the next attempt.".into(),
        after: "fn backoff_delay(&self) -> Duration {".into(),
    };
    assert_eq!(
        judge(&server, Duration::from_secs(10))
            .judge("q", &[candidate])
            .await,
        Relevance::Scored(vec![Some(0.9)])
    );
    let body: Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    let instructions = body["questions"]["relevant"]["instructions"]
        .as_str()
        .unwrap();
    assert!(instructions.contains("Judge those lines"), "{instructions}");
    assert!(instructions.contains("for context only"), "{instructions}");
}
