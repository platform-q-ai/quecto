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
    // The not-logged-in capture: `subtype:"success"` (which this type does
    // not even carry), `is_error:true`, `terminal_reason:"api_error"`, and a
    // synthetic assistant message whose `error` is `authentication_failed`.
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
        })
    );

    let assistant_error = TurnEnd::classify(&completed(), Some("rate_limit"));
    assert!(
        !assistant_error.is_completed(),
        "an assistant error fails the turn"
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
