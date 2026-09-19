use super::*;
use serde_json::json;

#[test]
fn a_request_is_sent_as_the_three_wire_fields() {
    let request = SessionSearchRequest {
        query: "a.*[b]".into(),
        scope: SessionListScope::Global,
        generation: 7,
    };
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({"query": "a.*[b]", "scope": "global", "generation": 7})
    );
}

#[test]
fn an_answer_carries_generation_rows_total_and_refusal() {
    let answer = parse_session_search(&json!({
        "generation": 9, "totalMatches": 41, "refused": null,
        "sessions": [
            {"key": "chat-1", "title": "One", "homeVersion": "h1-00000000000000aa",
             "executionPath": "/w/a", "homeState": "scoped", "resumeEligible": true},
            {"key": "cli:old", "title": "Old", "homeState": "legacy_unscoped"},
            {"key": "cli:untitled"},
        ],
    }));
    assert_eq!(
        (
            answer.generation,
            answer.total_matches,
            answer.refused.clone()
        ),
        (Some(9), 41, None)
    );
    let shown: Vec<_> = answer
        .sessions
        .iter()
        .map(|s| (s.key.as_str(), s.unscoped))
        .collect();
    assert_eq!(
        shown,
        [("chat-1", false), ("cli:old", true)],
        "an untitled row cannot be shown"
    );
    assert_eq!(
        answer.sessions[0].home_version.as_deref(),
        Some("h1-00000000000000aa")
    );
}

#[test]
fn an_answer_without_the_new_fields_is_read_defensively() {
    let answer = parse_session_search(&json!({"sessions": [{"key": "k", "title": "T"}]}));
    assert_eq!((answer.generation, answer.total_matches), (None, 1));
    let refused = parse_session_search(&json!({"generation": "9", "refused": "x".repeat(500)}));
    assert_eq!(refused.generation, None, "a generation is a number");
    assert_eq!(refused.refused.map(|r| r.len()), Some(200));
    assert!(parse_session_search(&json!(null)).sessions.is_empty());
}
