//! Regression tests for the dry run's own silent failures: a TypeSafe
//! overload (529) or Cloudflare error (520) that was not retried, an error
//! that could vanish instead of reaching the log, and the missing "waiting"
//! answers that made a correctly waiting agent look stalled.
use super::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn worker(endpoint: String, dir: PathBuf, role: AgentRole) -> Worker {
    Worker {
        client: reqwest::Client::new(),
        key: "test-key".into(),
        dir,
        role,
        recent: VecDeque::new(),
        endpoint,
        backoff: std::time::Duration::from_millis(1),
    }
}

fn ok_body() -> serde_json::Value {
    json!({"model": "jev-test", "answers": {"turn_end": {
        "type": "choice", "choice": "complete", "confidence": 0.9,
        "probabilities": {"complete": 0.9}
    }}, "usage": {"input_tokens": 1, "output_tokens": 1}})
}

#[test]
fn rate_limits_and_server_errors_are_retried_client_errors_are_not() {
    for status in [429, 500, 502, 520, 529, 599] {
        assert!(retryable_status(status), "{status}");
    }
    for status in [200, 400, 401, 403, 404, 422] {
        assert!(!retryable_status(status), "{status}");
    }
}

#[tokio::test]
async fn an_overload_then_a_cloudflare_error_are_retried_to_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(529))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(520))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().unwrap();
    let worker = worker(
        format!("{}/v1/systemone", server.uri()),
        tmp.path().into(),
        AgentRole::Root,
    );

    let answer = worker
        .ask(&json!({}))
        .await
        .expect("recovered after retries");

    assert_eq!(answer["model"], "jev-test");
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
}

#[tokio::test]
async fn a_client_error_is_final_after_one_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad question"))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().unwrap();
    let worker = worker(
        format!("{}/v1/systemone", server.uri()),
        tmp.path().into(),
        AgentRole::Root,
    );

    let error = worker.ask(&json!({})).await.unwrap_err();

    assert!(
        error.contains("HTTP 400") && error.contains("bad question"),
        "{error}"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_failed_judgment_is_logged_with_its_error_not_dropped() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("invalid key"))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().unwrap();
    let mut worker = worker(
        format!("{}/v1/systemone", server.uri()),
        tmp.path().into(),
        AgentRole::Root,
    );

    worker
        .judge(Job {
            ts: "1".into(),
            seq: 7,
            session_key: "s-1".into(),
            model: "m".into(),
            event: CommanderEvent::ToolError {
                turn: 1,
                tool: "bash".into(),
                arguments: "{}".into(),
                result: "exit 1".into(),
            },
        })
        .await;

    let log = std::fs::read_to_string(tmp.path().join("s-1.jsonl")).expect("a record was written");
    let record: serde_json::Value = serde_json::from_str(log.lines().next().unwrap()).unwrap();
    assert_eq!(record["seq"], 7);
    assert!(
        record["error"].as_str().unwrap().contains("HTTP 401"),
        "{record}"
    );
    assert!(record["answers"].is_null());
    assert_eq!(record["would_do"], json!({}));
}

#[test]
fn waiting_answers_map_to_no_action() {
    let answers = json!({
        "turn_end": {"choice": "waiting_on_others", "confidence": 0.95},
        "child_state": {"choice": "waiting_on_subagents", "confidence": 0.95}
    });
    let actions = Worker::would_do(&answers);
    assert_eq!(actions["5"], "none");
    assert_eq!(actions["6"], "plain_idle_note");
    // Below the confidence bar nothing is acted on either way.
    let unsure = json!({"turn_end": {"choice": "stopped_early", "confidence": 0.4}});
    assert_eq!(Worker::would_do(&unsure)["5"], "none_uncertain");
}

#[test]
fn a_child_turn_end_asks_the_waiting_questions() {
    let tmp = tempfile::tempdir().unwrap();
    let child = worker(
        "http://unused".into(),
        tmp.path().into(),
        AgentRole::Child {
            parent_id: Some("p".into()),
        },
    );
    let (_, questions, decisions) = child.build(&CommanderEvent::TurnEnd {
        turn: 1,
        prompt: "review".into(),
        final_text: "Both reviewers are still running; I will wait for their reports.".into(),
        stop_reason: None,
        tool_rounds: 1,
        ended_by: "final_response".into(),
        output_tokens: None,
        max_tokens: 100,
    });
    assert_eq!(decisions, ["5", "6", "22"]);
    assert!(
        questions["turn_end"]["criteria"]
            .get("waiting_on_others")
            .is_some()
    );
    assert!(
        questions["child_state"]["criteria"]
            .get("waiting_on_subagents")
            .is_some()
    );
}
