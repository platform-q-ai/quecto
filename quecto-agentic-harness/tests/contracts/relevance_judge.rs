//! Contract tests for the `RelevanceJudge` port (#2136), over its TypeSafe
//! adapter against a stand-in API: one score per candidate in order, an
//! unscored candidate where its request fails, `Unavailable` with the
//! reason when none can be scored, and an empty ranking for no candidates.

use std::time::Duration;

use quecto::application::search::ports::{Relevance, RelevanceCandidate, RelevanceJudge};
use quecto::infrastructure::search::typesafe_judge::TypeSafeJudge;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn candidate(location: &str) -> RelevanceCandidate {
    RelevanceCandidate {
        location: location.into(),
        text: "fn retry() {}".into(),
    }
}

fn scored(noul: f64) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_json(json!({"answers": {"relevant": {"type": "noul", "noul": noul}}}))
}

async fn judge(server: &MockServer) -> Box<dyn RelevanceJudge> {
    Box::new(
        TypeSafeJudge::new(
            &server.uri(),
            "contract-key".into(),
            "jev-latest",
            Duration::from_secs(5),
        )
        .expect("the client builds"),
    )
}

#[tokio::test]
async fn scores_come_back_one_per_candidate_in_order() {
    let server = MockServer::start().await;
    for (location, noul) in [("a.rs:1", 0.25), ("b.rs:1", 0.75)] {
        Mock::given(body_partial_json(
            json!({"state": {"candidate": {"location": location}}}),
        ))
        .respond_with(scored(noul))
        .mount(&server)
        .await;
    }
    let answer = judge(&server)
        .await
        .judge("q", &[candidate("a.rs:1"), candidate("b.rs:1")])
        .await;
    assert_eq!(answer, Relevance::Scored(vec![Some(0.25), Some(0.75)]));
}

#[tokio::test]
async fn a_failed_candidate_is_unscored_and_all_failing_is_unavailable() {
    let partly = MockServer::start().await;
    Mock::given(body_partial_json(
        json!({"state": {"candidate": {"location": "a.rs:1"}}}),
    ))
    .respond_with(scored(0.5))
    .mount(&partly)
    .await;
    Mock::given(body_partial_json(
        json!({"state": {"candidate": {"location": "b.rs:1"}}}),
    ))
    .respond_with(ResponseTemplate::new(500))
    .mount(&partly)
    .await;
    assert_eq!(
        judge(&partly)
            .await
            .judge("q", &[candidate("a.rs:1"), candidate("b.rs:1")])
            .await,
        Relevance::Partial(vec![Some(0.5), None], "TypeSafe answered HTTP 500".into())
    );
    let down = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&down)
        .await;
    assert!(matches!(
        judge(&down).await.judge("q", &[candidate("a.rs:1")]).await,
        Relevance::Unavailable(reason) if reason.contains("500")
    ));
}

#[tokio::test]
async fn no_candidates_is_an_empty_ranking() {
    let server = MockServer::start().await;
    assert_eq!(
        judge(&server).await.judge("q", &[]).await,
        Relevance::Scored(vec![])
    );
}
