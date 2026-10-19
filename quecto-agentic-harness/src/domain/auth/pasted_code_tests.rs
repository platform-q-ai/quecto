use super::*;

const STATE: &str = "expected";

#[test]
fn a_full_redirect_url_yields_its_code() {
    let url = "http://localhost:1455/auth/callback?code=ABC123&state=expected";
    assert_eq!(code_from_paste(url, STATE), Ok("ABC123".to_string()));
}

#[test]
fn a_bare_query_without_a_question_mark_must_carry_the_state_too() {
    // Carrying `code=` makes it query-shaped (#2591 review): no state, no
    // trust.
    assert_eq!(
        code_from_paste("a=1&code=LAST", STATE),
        Err(PasteRefusal::StateMissing)
    );
    assert_eq!(
        code_from_paste("a=1&code=LAST&state=expected", STATE),
        Ok("LAST".to_string())
    );
}

#[test]
fn a_state_parameter_alone_makes_input_query_shaped() {
    assert_eq!(
        code_from_paste("code=C&state=wrong", STATE),
        Err(PasteRefusal::StateMismatch)
    );
}

#[test]
fn a_query_shaped_paste_without_a_code_has_no_code() {
    assert_eq!(
        code_from_paste("?state=expected", STATE),
        Err(PasteRefusal::NoCode)
    );
    assert_eq!(
        code_from_paste("http://localhost/cb?code=&state=expected", STATE),
        Err(PasteRefusal::NoCode)
    );
    assert_eq!(
        code_from_paste("code=", STATE),
        Err(PasteRefusal::StateMissing)
    );
}

#[test]
fn a_bare_code_is_accepted_without_a_state() {
    // A bare code cannot carry state; some providers display one.
    assert_eq!(code_from_paste("ABC123", STATE), Ok("ABC123".to_string()));
}

#[test]
fn surrounding_whitespace_is_ignored() {
    assert_eq!(
        code_from_paste("  just-a-code \n", STATE),
        Ok("just-a-code".to_string())
    );
}

#[test]
fn an_empty_paste_has_no_code() {
    assert_eq!(code_from_paste("", STATE), Err(PasteRefusal::NoCode));
    assert_eq!(code_from_paste(" \n", STATE), Err(PasteRefusal::NoCode));
}

#[test]
fn a_url_with_the_expected_state_is_accepted() {
    let url = "http://127.0.0.1:56121/callback?code=ABC&state=expected";
    assert_eq!(code_from_paste(url, STATE), Ok("ABC".to_string()));
}

#[test]
fn a_url_with_another_state_is_refused() {
    let url = "http://127.0.0.1:56121/callback?code=ABC&state=wrong";
    assert_eq!(
        code_from_paste(url, STATE),
        Err(PasteRefusal::StateMismatch)
    );
}

#[test]
fn a_url_without_a_state_is_refused() {
    // Regression (PR #1087): URL-shaped input with no state must not slip through.
    let url = "http://127.0.0.1:56121/callback?code=ABC";
    assert_eq!(code_from_paste(url, STATE), Err(PasteRefusal::StateMissing));
}

#[test]
fn an_empty_state_counts_as_absent() {
    let url = "http://127.0.0.1:56121/callback?code=ABC&state=";
    assert_eq!(code_from_paste(url, STATE), Err(PasteRefusal::StateMissing));
}

#[test]
fn a_bare_query_fragment_must_carry_the_state_too() {
    assert_eq!(
        code_from_paste("?code=ABC", STATE),
        Err(PasteRefusal::StateMissing)
    );
    assert_eq!(
        code_from_paste("?code=ABC&state=expected", STATE),
        Ok("ABC".to_string())
    );
}

#[test]
fn the_state_is_url_decoded_before_it_is_compared() {
    let url = "http://127.0.0.1:56121/callback?code=ABC&state=exp%2Bected";
    assert_eq!(code_from_paste(url, "exp+ected"), Ok("ABC".to_string()));
}

#[test]
fn an_invalid_percent_escape_is_kept_raw() {
    assert_eq!(
        code_from_paste("?code=%ZZ&state=expected", STATE),
        Ok("%ZZ".to_string())
    );
}

#[test]
fn refusals_read_as_the_login_messages() {
    assert_eq!(
        PasteRefusal::StateMismatch.reason(),
        "state mismatch in pasted redirect URL"
    );
    assert_eq!(
        PasteRefusal::StateMissing.reason(),
        "pasted redirect URL is missing the state parameter"
    );
    assert_eq!(
        PasteRefusal::NoCode.reason(),
        "could not extract authorization code"
    );
}
