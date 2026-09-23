//! Regression tests for the dry run's own silent failures: a TypeSafe
//! overload (529) or Cloudflare error (520) that was not retried, an error
//! that could vanish instead of reaching the log, and the missing "waiting"
//! answers that made a correctly waiting agent look stalled.
use super::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn worker(endpoint: String, dir: PathBuf, role: AgentRole) -> Worker {
    Worker::new(
        reqwest::Client::new(),
        "test-key".into(),
        dir,
        role,
        endpoint,
        std::time::Duration::from_millis(1),
    )
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
            event: turn_end("Done."),
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
    let actions = Worker::would_do(&turn_end("x"), &AgentRole::Root, &answers);
    assert_eq!(actions["5"], "none");
    assert_eq!(actions["6"], "plain_idle_note");
    // Below the confidence bar nothing is acted on either way.
    let unsure = json!({"turn_end": {"choice": "stopped_early", "confidence": 0.4}});
    assert_eq!(
        Worker::would_do(&turn_end("x"), &AgentRole::Root, &unsure)["5"],
        "none_uncertain"
    );
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

// ---- Replay round (2026-09-23): fixes from the first live evaluation. ----

fn child() -> AgentRole {
    AgentRole::Child {
        parent_id: Some("p".into()),
    }
}

fn turn_end(text: &str) -> CommanderEvent {
    CommanderEvent::TurnEnd {
        turn: 1,
        prompt: "review".into(),
        final_text: text.into(),
        stop_reason: Some("end_turn".into()),
        tool_rounds: 1,
        ended_by: "final_response".into(),
        output_tokens: None,
        max_tokens: 100,
    }
}

fn tool_error(result: &str) -> CommanderEvent {
    CommanderEvent::ToolError {
        turn: 1,
        tool: "bash".into(),
        arguments: "{}".into(),
        result: result.into(),
    }
}

fn provider_failure(status: u16) -> CommanderEvent {
    CommanderEvent::ProviderFailure {
        turn: 1,
        provider: "p".into(),
        class: "transient".into(),
        http_status: Some(status),
        error: "Unable to verify access. Please try again.".into(),
        outcome: "transient_retry".into(),
        attempt: 1,
    }
}

#[tokio::test]
async fn a_cloudflare_403_page_is_retried_but_an_api_403_is_not() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(403).set_body_string("<!DOCTYPE html><html>Just a moment</html>"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().unwrap();
    let w = worker(
        format!("{}/x", server.uri()),
        tmp.path().into(),
        AgentRole::Root,
    );
    assert!(w.ask(&json!({})).await.is_ok());
    assert_eq!(server.received_requests().await.unwrap().len(), 2);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"error": "forbidden"})))
        .mount(&server)
        .await;
    let w = worker(
        format!("{}/x", server.uri()),
        tmp.path().into(),
        AgentRole::Root,
    );
    assert!(w.ask(&json!({})).await.unwrap_err().contains("HTTP 403"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_timed_out_request_is_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(ok_body())
                .set_delay(std::time::Duration::from_millis(800)),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().unwrap();
    let mut w = worker(
        format!("{}/x", server.uri()),
        tmp.path().into(),
        AgentRole::Root,
    );
    w.client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(200))
        .build()
        .unwrap();
    assert!(w.ask(&json!({})).await.is_ok(), "recovered after a timeout");
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[test]
fn a_child_is_asked_whether_its_parent_can_handle_it() {
    let tmp = tempfile::tempdir().unwrap();
    let w = worker("http://unused".into(), tmp.path().into(), child());
    let (_, questions, _) = w.build(&turn_end("Blocked: the PR head moved."));
    assert!(questions.contains_key("parent_can_handle"));
    let root = worker("http://unused".into(), tmp.path().into(), AgentRole::Root);
    let (_, questions, _) = root.build(&turn_end("Done."));
    assert!(!questions.contains_key("parent_can_handle"));
}

#[test]
fn a_child_escalation_the_parent_can_handle_stays_with_the_parent() {
    let answers = json!({
        "owner_needed": {"noul": 0.9},
        "urgency": {"score": 3.0},
        "parent_can_handle": {"noul": 0.8},
        "child_state": {"choice": "blocked", "confidence": 0.9}
    });
    let actions = Worker::would_do(&turn_end("x"), &child(), &answers);
    assert_eq!(actions["22"], "leave_to_parent");
    assert_eq!(actions["6"], "tell_parent_blocked_badge");
    assert!(actions.get("22_composed").is_none(), "{actions}");

    let unsure_parent = json!({
        "owner_needed": {"noul": 0.9},
        "urgency": {"score": 3.0},
        "parent_can_handle": {"noul": 0.3}
    });
    assert_eq!(
        Worker::would_do(&turn_end("x"), &child(), &unsure_parent)["22"],
        "interrupt_owner"
    );
}

#[test]
fn a_repeated_escalation_of_the_same_kind_interrupts_once_per_session() {
    let tmp = tempfile::tempdir().unwrap();
    let mut w = worker("http://unused".into(), tmp.path().into(), AgentRole::Root);
    let answers = json!({
        "owner_needed": {"noul": 0.9},
        "urgency": {"score": 3.0},
        "owner_kind": {"choice": "blocker", "confidence": 0.9}
    });
    let first = w.decide("s", &turn_end("x"), &answers);
    let second = w.decide("s", &turn_end("x"), &answers);
    let other_session = w.decide("t", &turn_end("x"), &answers);
    assert_eq!(first["22"], "interrupt_owner");
    assert_eq!(second["22"], "already_escalated");
    assert_eq!(other_session["22"], "interrupt_owner");
}

#[tokio::test]
async fn every_tool_error_is_judged_and_a_repeated_one_also_asks_about_progress() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().unwrap();
    let mut w = worker(
        format!("{}/x", server.uri()),
        tmp.path().into(),
        AgentRole::Root,
    );
    let job = |seq, result: &str| Job {
        ts: "1".into(),
        seq,
        session_key: "s".into(),
        model: "m".into(),
        event: tool_error(result),
    };
    w.judge(job(1, "test failed: expected 3")).await;
    w.judge(job(2, "error: build failed")).await;
    w.judge(job(3, "error: build failed")).await;
    w.judge(job(4, "error: build failed")).await;
    assert_eq!(server.received_requests().await.unwrap().len(), 4);

    let log = std::fs::read_to_string(tmp.path().join("s.jsonl")).unwrap();
    let records: Vec<serde_json::Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(records[0]["decisions"], json!(["22"]));
    assert_eq!(records[3]["decisions"], json!(["22", "stall"]));
}

#[test]
fn a_tool_error_reaches_the_owner_only_when_it_is_urgent() {
    let event = tool_error("gh: HTTP 401 Bad credentials");
    let digest = json!({"owner_needed": {"noul": 0.6}});
    assert_eq!(
        Worker::would_do(&event, &AgentRole::Root, &digest)["22"],
        "none",
        "routine failures are not digest material"
    );
    let urgent = json!({"owner_needed": {"noul": 0.9}, "urgency": {"score": 3.0}});
    assert_eq!(
        Worker::would_do(&event, &AgentRole::Root, &urgent)["22"],
        "interrupt_owner"
    );
}

#[test]
fn configuration_stops_the_run_only_on_an_owner_fixable_status_or_high_confidence() {
    let config =
        |conf: f64| json!({"provider_error": {"choice": "configuration", "confidence": conf}});
    let root = AgentRole::Root;
    assert_eq!(
        Worker::would_do(&provider_failure(503), &root, &config(0.66))["12"],
        "keep_rule_based_handling"
    );
    assert_eq!(
        Worker::would_do(&provider_failure(503), &root, &config(0.9))["12"],
        "stop_and_tell_owner"
    );
    assert_eq!(
        Worker::would_do(&provider_failure(401), &root, &config(0.66))["12"],
        "stop_and_tell_owner"
    );
}

#[tokio::test]
async fn a_thrashing_agent_is_asked_whether_it_is_stuck() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().unwrap();
    let mut w = worker(format!("{}/x", server.uri()), tmp.path().into(), child());
    for (seq, result) in ["a", "b", "c", "d", "e", "f"].into_iter().enumerate() {
        w.judge(Job {
            ts: format!("{}", 100 + seq),
            seq: seq as u64,
            session_key: "s".into(),
            model: "m".into(),
            event: tool_error(&format!("oldText not found {result}")),
        })
        .await;
    }
    let requests = server.received_requests().await.unwrap();
    let asked: Vec<bool> = requests
        .iter()
        .map(|r| {
            let body: serde_json::Value = serde_json::from_slice(&r.body).unwrap();
            body["questions"].get("progress").is_some()
        })
        .collect();
    assert_eq!(asked, [false, false, false, false, false, true]);
    let last: serde_json::Value = serde_json::from_slice(&requests[5].body).unwrap();
    assert_eq!(
        last["state"]["recent_tool_calls"].as_array().unwrap().len(),
        6
    );
}

#[tokio::test]
async fn a_successful_tool_call_is_logged_without_a_judgment() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let mut w = worker(format!("{}/x", server.uri()), tmp.path().into(), child());
    w.judge(Job {
        ts: "1".into(),
        seq: 0,
        session_key: "s".into(),
        model: "m".into(),
        event: CommanderEvent::ToolOk {
            turn: 1,
            tool: "bash".into(),
        },
    })
    .await;
    assert!(server.received_requests().await.unwrap().is_empty());
    let log = std::fs::read_to_string(tmp.path().join("s.jsonl")).unwrap();
    assert!(log.contains("\"tool_ok\""), "{log}");
}

#[tokio::test]
async fn a_turn_end_starts_the_stall_window_afresh() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().unwrap();
    let mut w = worker(format!("{}/x", server.uri()), tmp.path().into(), child());
    let mut seq = 0;
    let mut send = |event: CommanderEvent| {
        seq += 1;
        Job {
            ts: format!("{}", 100 + seq),
            seq,
            session_key: "s".into(),
            model: "m".into(),
            event,
        }
    };
    for n in 0..5 {
        w.judge(send(tool_error(&format!("red {n}")))).await;
    }
    w.judge(send(turn_end("Partial; continuing."))).await;
    w.judge(send(tool_error("new attempt fails"))).await;
    let requests = server.received_requests().await.unwrap();
    let last: serde_json::Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
    assert!(
        last["questions"].get("progress").is_none(),
        "five failures before the turn end and one after are not a stall"
    );
}

#[test]
fn only_a_clear_yes_leaves_an_urgent_matter_with_the_parent() {
    let answers = |parent: f64| {
        json!({
            "owner_needed": {"noul": 0.92},
            "urgency": {"score": 3.0},
            "parent_can_handle": {"noul": parent}
        })
    };
    assert_eq!(
        Worker::would_do(&turn_end("x"), &child(), &answers(0.51))["22"],
        "interrupt_owner"
    );
    assert_eq!(
        Worker::would_do(&turn_end("x"), &child(), &answers(0.75))["22"],
        "leave_to_parent"
    );
}

#[test]
fn a_sub_agent_notice_asks_whether_this_agent_can_handle_it() {
    let tmp = tempfile::tempdir().unwrap();
    let root = worker("http://unused".into(), tmp.path().into(), AgentRole::Root);
    let notice = CommanderEvent::SubagentNotice {
        child: "review".into(),
        child_uuid: None,
        notice: "Agent 'review' stalled: idle with workflow still active at 2/5.".into(),
        detail: None,
    };
    let (_, questions, _) = root.build(&notice);
    assert!(questions.contains_key("parent_can_handle"));
    let answers = json!({"owner_needed": {"noul": 0.8}, "urgency": {"score": 2.0}, "parent_can_handle": {"noul": 0.9}});
    assert_eq!(
        Worker::would_do(&notice, &AgentRole::Root, &answers)["22"],
        "leave_to_parent"
    );
}

#[tokio::test]
async fn an_identical_failure_repeated_three_times_asks_the_stall_question() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_body()))
        .mount(&server)
        .await;
    let tmp = tempfile::tempdir().unwrap();
    let mut w = worker(format!("{}/x", server.uri()), tmp.path().into(), child());
    for seq in 0..3 {
        w.judge(Job {
            ts: format!("{}", 100 + seq),
            seq,
            session_key: "s".into(),
            model: "m".into(),
            event: tool_error("error[E0433]: failed to resolve"),
        })
        .await;
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    let body: serde_json::Value = serde_json::from_slice(&requests[2].body).unwrap();
    assert!(body["questions"].get("progress").is_some(), "{body}");
}

#[test]
fn an_html_error_page_never_stops_the_run_as_configuration() {
    let page = CommanderEvent::ProviderFailure {
        turn: 1,
        provider: "openrouter".into(),
        class: "auth".into(),
        http_status: Some(403),
        error: "provider error: HTTP 403 from OpenRouter: <!DOCTYPE html><html><title>Just a moment...</title></html>".into(),
        outcome: "terminal".into(),
        attempt: 0,
    };
    let answers = json!({"provider_error": {"choice": "configuration", "confidence": 0.99}});
    assert_eq!(
        Worker::would_do(&page, &AgentRole::Root, &answers)["12"],
        "keep_rule_based_handling"
    );
}

#[test]
fn a_terminal_provider_failure_on_the_root_reaches_the_owner() {
    let mut failure = provider_failure(503);
    if let CommanderEvent::ProviderFailure { outcome, .. } = &mut failure {
        *outcome = "terminal".into();
    }
    let answers = json!({
        "provider_error": {"choice": "transient", "confidence": 0.9},
        "owner_needed": {"noul": 0.2}
    });
    let root = Worker::would_do(&failure, &AgentRole::Root, &answers);
    assert_eq!(root["22_composed"], "promote_in_tui", "{root}");
    let retried = Worker::would_do(&provider_failure(503), &AgentRole::Root, &answers);
    assert!(retried.get("22_composed").is_none(), "{retried}");
}

#[test]
fn a_policy_refusal_gets_a_digest_line() {
    let answers = json!({
        "provider_error": {"choice": "policy_refusal", "confidence": 0.9},
        "owner_needed": {"noul": 0.2}
    });
    let actions = Worker::would_do(&provider_failure(400), &AgentRole::Root, &answers);
    assert_eq!(actions["22_composed"], "add_to_digest", "{actions}");
}

#[test]
fn a_stuck_root_is_shown_to_the_owner_and_a_stuck_child_to_its_parent() {
    let answers = json!({
        "progress": {"choice": "stuck", "confidence": 0.9},
        "owner_needed": {"noul": 0.55}
    });
    let event = tool_error("oldText not found");
    assert_eq!(
        Worker::would_do(&event, &AgentRole::Root, &answers)["22_composed"],
        "promote_in_tui"
    );
    assert_eq!(
        Worker::would_do(&event, &child(), &answers)["22_composed"],
        "leave_to_parent"
    );
}

#[test]
fn a_second_crash_in_a_session_is_promoted() {
    let tmp = tempfile::tempdir().unwrap();
    let mut w = worker("http://unused".into(), tmp.path().into(), AgentRole::Root);
    let crash = |child: &str| CommanderEvent::SubagentNotice {
        child: child.into(),
        child_uuid: None,
        notice: format!("Agent '{child}' exited unexpectedly (killed by signal 9)"),
        detail: None,
    };
    let answers = json!({"owner_needed": {"noul": 0.3}, "parent_can_handle": {"noul": 0.9}});
    let first = w.decide("s", &crash("a"), &answers);
    let second = w.decide("s", &crash("a-retry"), &answers);
    assert!(first.get("22_composed").is_none(), "{first}");
    assert_eq!(second["22_composed"], "promote_in_tui");
}

#[tokio::test]
async fn without_a_key_events_are_recorded_for_later_judgment_not_sent() {
    let server = MockServer::start().await;
    let tmp = tempfile::tempdir().unwrap();
    let mut w = worker(format!("{}/x", server.uri()), tmp.path().into(), child());
    w.record_only = true;
    for (seq, event) in [turn_end("Blocked."), tool_error("boom")]
        .into_iter()
        .enumerate()
    {
        w.judge(Job {
            ts: format!("{}", 100 + seq),
            seq: seq as u64,
            session_key: "in-container".into(),
            model: "m".into(),
            event,
        })
        .await;
    }
    assert!(server.received_requests().await.unwrap().is_empty());
    let log = std::fs::read_to_string(tmp.path().join("in-container.jsonl")).unwrap();
    let records: Vec<serde_json::Value> = log
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(records.len(), 2);
    for record in &records {
        assert_eq!(record["mode"], "record", "{record}");
        assert!(record["answers"].is_null());
        assert!(record["event"]["kind"].is_string());
        assert!(record["agent_role"].as_str().unwrap().starts_with("Child"));
    }
}
