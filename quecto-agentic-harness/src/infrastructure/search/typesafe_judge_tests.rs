use super::*;
use wiremock::matchers::{body_partial_json, header, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn candidate(location: &str) -> RelevanceCandidate {
    RelevanceCandidate {
        location: location.into(),
        text: format!("text at {location}"),
    }
}

fn judge(server: &MockServer, timeout: Duration) -> TypeSafeJudge {
    TypeSafeJudge::new(&server.uri(), "test-key".into(), "jev-latest", timeout)
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
    assert_eq!(judged, Relevance::Scored(vec![Some(0.7), None]));

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
