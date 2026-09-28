//! Turn classification and usage accounting (#2285).

use super::*;

fn completed() -> ResultEvent {
    ResultEvent {
        is_error: Some(false),
        terminal_reason: Some("completed".into()),
        stop_reason: Some("end_turn".into()),
        ..ResultEvent::default()
    }
}

// Mapping row: `result.is_error`, `terminal_reason`, `api_error_status`,
// assistant `error` → end classification ("never use `subtype` alone").
#[test]
fn a_success_subtype_with_is_error_is_a_failed_turn() {
    // The not-logged-in shape: the CLI's `subtype` says "success" (a field
    // `ResultEvent` deliberately does not model), yet `is_error` is true and
    // `terminal_reason` is "api_error", after a synthetic assistant message
    // whose `error` is "authentication_failed".
    let not_logged_in = ResultEvent {
        is_error: Some(true),
        terminal_reason: Some("api_error".into()),
        api_error_status: None,
        result_text: Some("Not logged in · Please run /login".into()),
        ..ResultEvent::default()
    };
    assert_eq!(
        TurnEnd::classify(&not_logged_in, Some("authentication_failed")),
        TurnEnd::Failed(TurnFailure {
            terminal_reason: Some("api_error".into()),
            api_error_status: None,
            assistant_error: Some("authentication_failed".into()),
            errors: Vec::new(),
            kind: FailureKind::Error,
        })
    );
}

#[test]
fn completed_requires_terminal_reason_completed_and_no_error() {
    assert_eq!(TurnEnd::classify(&completed(), None), TurnEnd::Completed);
    assert!(TurnEnd::classify(&completed(), None).is_completed());

    let errored = ResultEvent {
        is_error: Some(true),
        ..completed()
    };
    assert!(!TurnEnd::classify(&errored, None).is_completed());

    let api_error = ResultEvent {
        terminal_reason: Some("api_error".into()),
        api_error_status: Some(529),
        ..completed()
    };
    assert_eq!(
        TurnEnd::classify(&api_error, None),
        TurnEnd::Failed(TurnFailure {
            terminal_reason: Some("api_error".into()),
            api_error_status: Some(529),
            assistant_error: None,
            errors: Vec::new(),
            kind: FailureKind::Error,
        })
    );
}

#[test]
fn an_assistant_error_does_not_fail_a_result_that_says_completed() {
    // The CLI's result is followed; the projection records the error as a
    // warning.
    assert_eq!(
        TurnEnd::classify(&completed(), Some("rate_limit")),
        TurnEnd::Completed
    );
}

#[test]
fn an_error_result_carries_its_errors_into_the_failure() {
    let max_turns = ResultEvent {
        is_error: Some(true),
        terminal_reason: Some("max_turns".into()),
        errors: vec!["Reached maximum number of turns (5)".into()],
        ..ResultEvent::default()
    };
    let TurnEnd::Failed(failure) = TurnEnd::classify(&max_turns, None) else {
        panic!("an error result is a failed turn");
    };
    assert_eq!(failure.errors, vec!["Reached maximum number of turns (5)"]);
    let described = failure.describe();
    assert!(described.contains("max_turns"), "{described}");
    assert!(
        described.contains("Reached maximum number of turns (5)"),
        "{described}"
    );
}

#[test]
fn a_failure_describes_every_signal_it_has() {
    let bare = TurnFailure::default().describe();
    assert_eq!(bare, "the turn failed");
    let full = TurnFailure {
        terminal_reason: Some("api_error".into()),
        api_error_status: Some(401),
        assistant_error: Some("authentication_failed".into()),
        errors: vec!["one".into(), "two".into()],
        kind: FailureKind::Error,
    }
    .describe();
    assert_eq!(
        full,
        "the turn failed (terminal_reason: api_error; api_error_status: 401; \
         assistant error: authentication_failed): one; two"
    );
}

#[test]
fn a_result_missing_either_field_is_a_failed_turn() {
    let no_reason = ResultEvent {
        terminal_reason: None,
        ..completed()
    };
    let no_error_flag = ResultEvent {
        is_error: None,
        ..completed()
    };
    assert!(!TurnEnd::classify(&no_reason, None).is_completed());
    assert!(!TurnEnd::classify(&no_error_flag, None).is_completed());
    assert!(!TurnEnd::classify(&ResultEvent::default(), None).is_completed());
}

#[test]
fn an_abort_is_a_failed_turn_classified_as_an_abort() {
    for reason in ABORT_TERMINAL_REASONS {
        let stopped = ResultEvent {
            is_error: Some(false),
            terminal_reason: Some(reason.to_string()),
            ..ResultEvent::default()
        };
        let TurnEnd::Failed(failure) = TurnEnd::classify(&stopped, None) else {
            panic!("{reason} does not complete a turn");
        };
        assert_eq!(failure.kind, FailureKind::Aborted, "{reason}");
        assert_eq!(
            failure.describe(),
            format!("the turn was aborted (terminal_reason: {reason})")
        );
    }
    let errored = ResultEvent {
        is_error: Some(true),
        terminal_reason: Some("api_error".into()),
        ..ResultEvent::default()
    };
    let TurnEnd::Failed(failure) = TurnEnd::classify(&errored, None) else {
        panic!("an API error fails the turn");
    };
    assert_eq!(failure.kind, FailureKind::Error);
    let TurnEnd::Failed(unknown) = TurnEnd::classify(&ResultEvent::default(), None) else {
        panic!("a bare result fails the turn");
    };
    assert_eq!(unknown.kind, FailureKind::Error);
}
