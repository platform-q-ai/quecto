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
    let untyped = parse_session_search(&json!({"generation": "9", "totalMatches": 4}));
    assert_eq!(untyped.generation, None, "a generation is a number");
    let refused = parse_session_search(&json!({"generation": 9, "refused": "x".repeat(500)}));
    assert_eq!(refused.refused.map(|r| r.len()), Some(200));
    assert!(parse_session_search(&json!(null)).sessions.is_empty());
}

/// R1-T2 / R1-T9: the truncation flag, the echoed scope and why a row matched
/// reach the picker; untrusted text is made safe and bounded on the way.
#[test]
fn an_answer_carries_truncation_the_echoed_scope_and_why_each_row_matched() {
    let answer = parse_session_search(&json!({
        "generation": 3, "scope": "global", "totalMatches": 5200, "truncated": true,
        "sessions": [
            {"key": "chat-1", "title": "One", "repositoryLabel": "que\u{1b}[2Jcto\u{202e}",
             "matched": ["title", "repository", "path", "key", "bogus", 7]},
            {"key": "chat-2", "title": "Two", "repositoryLabel": null},
        ],
    }));
    assert!(answer.truncated);
    assert_eq!(
        (answer.total_matches, answer.scope),
        (5200, Some(SessionListScope::Global))
    );
    let label = answer.sessions[0].repository_label.as_deref().unwrap();
    assert!(
        label.starts_with("que") && !label.contains(['\u{1b}', '\u{202e}']),
        "{label:?}"
    );
    assert_eq!(
        answer.sessions[0].matched,
        ["title", "repository", "path", "key"]
    );
    assert_eq!(
        (
            &answer.sessions[1].repository_label,
            answer.sessions[1].matched.len()
        ),
        (&None, 0)
    );
    // Absent or malformed: not truncated, no scope — never a guess. The flag
    // is also inferred from the counts, for a harness that sent only those.
    let bare = parse_session_search(
        &json!({"sessions": [{"key": "k", "title": "t"}], "scope": "everywhere"}),
    );
    assert_eq!(
        (bare.truncated, bare.scope, bare.total_matches),
        (false, None, 1)
    );
    let counted =
        parse_session_search(&json!({"sessions": [{"key": "k", "title": "t"}], "totalMatches": 9}));
    assert!(counted.truncated);
}
